use crate::bitmap::BitMap;
use crate::cache::MetaCache;
use crate::device::BlockDevice;
use crate::dir;
use crate::error::FsError;
use crate::extent::{self, Extent};
use crate::inode::{
    self, Inode, KIND_DIRECTORY, KIND_FREE, KIND_REGULAR, MODE_DIRECTORY, MODE_FILE,
};
use crate::path::{self, PathParts};
use crate::spec::{
    BLOCK_BYTES, Geometry, INODE_SIZE, MAX_OPEN, RESERVED_PREFIX_BLOCKS, ROOT_INODE,
    SUPERBLOCK_BLOCK,
};
use crate::superblock::{Superblock, CHECK_CLEAN, CHECK_UNCLEAN, UUID_BYTES, VOLUME_NAME_BYTES};
use alloc::vec::Vec;

pub struct FormatParams<'a> {
    pub bytes_per_inode: u32,
    pub uuid: [u8; UUID_BYTES],
    pub volume_name: &'a [u8],
    pub created_at: u64,
}

pub struct FileSystem<D: BlockDevice> {
    dev: D,
    sb: Superblock,
    geometry: Geometry,
    block_bitmap: BitMap,
    inode_bitmap: BitMap,
    cache: MetaCache,
    block_hint: u64,
    inode_hint: u64,
    opens: [Option<OpenEntry>; MAX_OPEN],
    unclean_previous: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct OpenEntry {
    inode_no: u32,
    generation: u32,
    kind: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Handle {
    slot: u8,
    generation: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Regular,
    Directory,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stat {
    pub inode: u32,
    pub generation: u32,
    pub kind: Kind,
    pub size: u64,
    pub blocks: u32,
    pub mtime: u64,
}

impl<D: BlockDevice> FileSystem<D> {
    pub fn format(mut dev: D, params: &FormatParams) -> Result<FileSystem<D>, FsError> {
        if params.volume_name.len() > VOLUME_NAME_BYTES {
            return Err(FsError::NameTooLong);
        }
        let geometry = Geometry::plan(dev.block_count(), params.bytes_per_inode)?;
        let mut sb = Superblock::from_geometry(&geometry)?;
        sb.uuid = params.uuid;
        sb.volume_name[..params.volume_name.len()].copy_from_slice(params.volume_name);
        sb.created_at = params.created_at;

        let zero = [0u8; BLOCK_BYTES];
        for block in 0..RESERVED_PREFIX_BLOCKS {
            write(&mut dev, block, &zero)?;
        }
        for offset in 0..geometry.inode_table_blocks {
            write(&mut dev, geometry.inode_table_start + offset, &zero)?;
        }

        let mut block_bitmap = BitMap::empty(geometry.total_blocks);
        block_bitmap.use_range(0, geometry.data_start);
        block_bitmap.set_used(geometry.backup_superblock);
        block_bitmap.set_used(geometry.data_start);
        block_bitmap.store(&mut dev, geometry.block_bitmap_start)?;

        let mut inode_bitmap = BitMap::empty(geometry.inode_count);
        inode_bitmap.set_used(0);
        inode_bitmap.set_used(ROOT_INODE as u64);
        inode_bitmap.store(&mut dev, geometry.inode_bitmap_start)?;

        let mut root_dir_block = [0u8; BLOCK_BYTES];
        dir::init_dot_entries(&mut root_dir_block, ROOT_INODE, ROOT_INODE);
        write(&mut dev, geometry.data_start, &root_dir_block)?;

        let root_inode = Inode {
            kind: KIND_DIRECTORY,
            mode: MODE_DIRECTORY,
            links: 2,
            generation: 0,
            size: BLOCK_BYTES as u64,
            mtime: 0,
            blocks_used: 1,
            extent_count: 1,
            spill_block: 0,
        };
        let root_slot = ROOT_INODE as usize * INODE_SIZE;
        let mut inode_bytes = [0u8; INODE_SIZE];
        inode::encode(
            &root_inode,
            &[Extent {
                start: geometry.data_start as u32,
                length: 1,
            }],
            &mut inode_bytes,
        );
        let mut table_block = zero;
        let slot_in_block = root_slot % BLOCK_BYTES;
        table_block[slot_in_block..slot_in_block + INODE_SIZE].copy_from_slice(&inode_bytes);
        write(
            &mut dev,
            geometry.inode_table_start + (root_slot / BLOCK_BYTES) as u64,
            &table_block,
        )?;

        sb.free_blocks = block_bitmap.free_count();
        sb.free_inodes = inode_bitmap.free_count() as u32;
        let mut sb_block = [0u8; BLOCK_BYTES];
        sb.encode(&mut sb_block);
        write(&mut dev, SUPERBLOCK_BLOCK, &sb_block)?;
        write(&mut dev, geometry.backup_superblock, &sb_block)?;
        dev.flush().map_err(FsError::Io)?;

        Ok(FileSystem {
            dev,
            sb,
            geometry,
            block_bitmap,
            inode_bitmap,
            cache: MetaCache::new(),
            block_hint: geometry.data_start,
            inode_hint: 2,
            opens: [None; MAX_OPEN],
            unclean_previous: false,
        })
    }

    pub fn mount(mut dev: D) -> Result<FileSystem<D>, FsError> {
        let device_blocks = dev.block_count();
        let (mut sb, from_backup) = read_superblock(&mut dev, device_blocks)?;
        if sb.total_blocks > device_blocks {
            return Err(FsError::Corrupt);
        }
        let data_start = sb.inode_table_start + sb.inode_table_blocks as u64;
        let backup_superblock = sb.total_blocks - 1;
        if data_start >= backup_superblock {
            return Err(FsError::Corrupt);
        }
        let geometry = Geometry {
            total_blocks: sb.total_blocks,
            inode_count: sb.inode_count as u64,
            block_bitmap_start: sb.block_bitmap_start,
            block_bitmap_blocks: sb.block_bitmap_blocks as u64,
            inode_bitmap_start: sb.inode_bitmap_start,
            inode_bitmap_blocks: sb.inode_bitmap_blocks as u64,
            inode_table_start: sb.inode_table_start,
            inode_table_blocks: sb.inode_table_blocks as u64,
            data_start,
            data_blocks: backup_superblock - data_start,
            backup_superblock,
        };
        let block_bitmap = BitMap::load(
            &mut dev,
            geometry.block_bitmap_start,
            geometry.block_bitmap_blocks,
            geometry.total_blocks,
        )?;
        let inode_bitmap = BitMap::load(
            &mut dev,
            geometry.inode_bitmap_start,
            geometry.inode_bitmap_blocks,
            geometry.inode_count,
        )?;
        let unclean_previous = from_backup || sb.mount_state == CHECK_UNCLEAN;
        sb.free_blocks = block_bitmap.free_count();
        sb.free_inodes = inode_bitmap.free_count() as u32;
        sb.mount_state = CHECK_UNCLEAN;
        let mut sb_block = [0u8; BLOCK_BYTES];
        sb.encode(&mut sb_block);
        write(&mut dev, SUPERBLOCK_BLOCK, &sb_block)?;
        let block_hint = if (geometry.data_start..geometry.backup_superblock).contains(&(sb.alloc_hint as u64))
        {
            sb.alloc_hint as u64
        } else {
            geometry.data_start
        };
        Ok(FileSystem {
            dev,
            sb,
            geometry,
            block_bitmap,
            inode_bitmap,
            cache: MetaCache::new(),
            block_hint,
            inode_hint: 2,
            opens: [None; MAX_OPEN],
            unclean_previous,
        })
    }

    pub fn unmount(mut self) -> Result<D, FsError> {
        self.cache.flush(&mut self.dev)?;
        let mut sb = self.sb;
        sb.mount_state = CHECK_CLEAN;
        sb.free_blocks = self.block_bitmap.free_count();
        sb.free_inodes = self.inode_bitmap.free_count() as u32;
        sb.alloc_hint = self.block_hint as u32;
        let mut sb_block = [0u8; BLOCK_BYTES];
        sb.encode(&mut sb_block);
        write(&mut self.dev, SUPERBLOCK_BLOCK, &sb_block)?;
        write(&mut self.dev, self.geometry.backup_superblock, &sb_block)?;
        self.dev.flush().map_err(FsError::Io)?;
        Ok(self.dev)
    }

    pub fn superblock(&self) -> &Superblock {
        &self.sb
    }

    pub fn unclean_previous(&self) -> bool {
        self.unclean_previous
    }

    pub fn free_blocks(&self) -> u64 {
        self.block_bitmap.free_count()
    }

    pub fn free_inodes(&self) -> u64 {
        self.inode_bitmap.free_count()
    }

    pub fn lookup(&mut self, path_bytes: &[u8]) -> Result<Stat, FsError> {
        let parts = self::path::parse(path_bytes)?;
        let inode_no = self.resolve(&parts)?;
        let (inode, _) = self.load_inode(inode_no)?;
        Ok(stat_of(inode_no, &inode))
    }

    pub fn open(&mut self, path_bytes: &[u8]) -> Result<Handle, FsError> {
        let parts = self::path::parse(path_bytes)?;
        let inode_no = self.resolve(&parts)?;
        self.open_handle(inode_no)
    }

    pub fn create(&mut self, path_bytes: &[u8]) -> Result<Handle, FsError> {
        let parts = self::path::parse(path_bytes)?;
        if parts.is_root() {
            return Err(FsError::AlreadyExists);
        }
        let parent_no = self.resolve_prefix(&parts, parts.len() - 1)?;
        let name = parts.get(parts.len() - 1).ok_or(FsError::InvalidPath)?;
        let (mut parent_inode, mut parent_extents) = self.load_inode(parent_no)?;
        if parent_inode.kind != KIND_DIRECTORY {
            return Err(FsError::NotADirectory);
        }
        if self
            .dir_find(&parent_inode, &parent_extents, name)?
            .is_some()
        {
            return Err(FsError::AlreadyExists);
        }
        let inode_no = self.alloc_inode()?;
        let generation = self.slot_generation(inode_no)?;
        let mut inode = Inode::new(KIND_REGULAR, MODE_FILE, generation);
        inode.mtime = self.sb.txg;
        self.store_inode(inode_no, &inode, &[])?;
        self.dir_add_entry(&mut parent_inode, &mut parent_extents, name, KIND_REGULAR, inode_no)?;
        parent_inode.mtime = self.sb.txg;
        self.store_inode(parent_no, &parent_inode, &parent_extents)?;
        self.open_handle(inode_no)
    }

    pub fn read(&mut self, handle: Handle, offset: u64, buf: &mut [u8]) -> Result<usize, FsError> {
        let inode_no = self.validate_handle(handle)?;
        let (inode, extents) = self.load_inode(inode_no)?;
        if inode.kind == KIND_DIRECTORY {
            return Err(FsError::IsADirectory);
        }
        if offset >= inode.size {
            return Ok(0);
        }
        let available = (inode.size - offset).min(buf.len() as u64);
        let mut done = 0u64;
        let mut block = [0u8; BLOCK_BYTES];
        while done < available {
            let file_offset = offset + done;
            let file_block = file_offset / BLOCK_BYTES as u64;
            let (extent, inner) = extent::locate(&extents, file_block).ok_or(FsError::Corrupt)?;
            let physical = extent.start as u64 + inner;
            let block_off = (file_offset % BLOCK_BYTES as u64) as usize;
            let chunk = (BLOCK_BYTES - block_off).min((available - done) as usize);
            dev_read(&mut self.dev, physical, &mut block)?;
            let src = block_off..block_off + chunk;
            let dst = done as usize..done as usize + chunk;
            buf[dst].copy_from_slice(&block[src]);
            done += chunk as u64;
        }
        Ok(available as usize)
    }

    pub fn write(
        &mut self,
        handle: Handle,
        offset: u64,
        data: &[u8],
    ) -> Result<usize, FsError> {
        if data.is_empty() {
            return Ok(0);
        }
        let inode_no = self.validate_handle(handle)?;
        let (mut inode, mut extents) = self.load_inode(inode_no)?;
        if inode.kind == KIND_DIRECTORY {
            return Err(FsError::IsADirectory);
        }
        let end = offset.checked_add(data.len() as u64).ok_or(FsError::FileTooLarge)?;
        let needed_blocks = ceil_blocks(end);
        if needed_blocks > inode.blocks_used as u64 {
            let extra = needed_blocks - inode.blocks_used as u64;
            let runs = self.alloc_blocks(extra)?;
            self.zero_blocks(&runs)?;
            for run in runs {
                if !extent::append(&mut extents, run.0, run.1) {
                    return Err(FsError::FileTooLarge);
                }
            }
            inode.blocks_used = extent::total_blocks(&extents) as u32;
        }
        if extents.len() > u16::MAX as usize {
            return Err(FsError::FileTooLarge);
        }
        let mut block = [0u8; BLOCK_BYTES];
        let mut done = 0usize;
        while done < data.len() {
            let file_offset = offset + done as u64;
            let file_block = file_offset / BLOCK_BYTES as u64;
            let (extent, inner) = extent::locate(&extents, file_block).ok_or(FsError::Corrupt)?;
            let physical = extent.start as u64 + inner;
            let block_off = (file_offset % BLOCK_BYTES as u64) as usize;
            let chunk = (BLOCK_BYTES - block_off).min(data.len() - done);
            let start_in_data = done;
            if chunk != BLOCK_BYTES || block_off != 0 {
                dev_read(&mut self.dev, physical, &mut block)?;
            }
            let from = start_in_data..start_in_data + chunk;
            block[block_off..block_off + chunk].copy_from_slice(&data[from]);
            dev_write(&mut self.dev, physical, &block)?;
            done += chunk;
        }
        if end > inode.size {
            inode.size = end;
        }
        inode.mtime = self.sb.txg;
        self.store_inode(inode_no, &inode, &extents)?;
        Ok(data.len())
    }

    pub fn stat(&mut self, handle: Handle) -> Result<Stat, FsError> {
        let inode_no = self.validate_handle(handle)?;
        let (inode, _) = self.load_inode(inode_no)?;
        Ok(stat_of(inode_no, &inode))
    }

    pub fn close(&mut self, handle: Handle) -> Result<(), FsError> {
        self.validate_handle(handle)?;
        self.opens[handle.slot as usize] = None;
        Ok(())
    }

    fn open_handle(&mut self, inode_no: u32) -> Result<Handle, FsError> {
        let (inode, _) = self.load_inode(inode_no)?;
        if inode.kind == KIND_FREE {
            return Err(FsError::NotFound);
        }
        let slot = self
            .opens
            .iter()
            .position(|entry| entry.is_none())
            .ok_or(FsError::TooManyOpen)?;
        self.opens[slot] = Some(OpenEntry {
            inode_no,
            generation: inode.generation,
            kind: inode.kind,
        });
        Ok(Handle {
            slot: slot as u8,
            generation: inode.generation,
        })
    }

    fn validate_handle(&self, handle: Handle) -> Result<u32, FsError> {
        let entry = self.opens.get(handle.slot as usize).and_then(|e| e.as_ref());
        match entry {
            Some(entry) if entry.generation == handle.generation => Ok(entry.inode_no),
            _ => Err(FsError::BadHandle),
        }
    }

    fn resolve(&mut self, parts: &PathParts) -> Result<u32, FsError> {
        self.resolve_prefix(parts, parts.len())
    }

    fn resolve_prefix(&mut self, parts: &PathParts, stop: usize) -> Result<u32, FsError> {
        let mut current = ROOT_INODE;
        for index in 0..stop {
            let (inode, extents) = self.load_inode(current)?;
            if inode.kind != KIND_DIRECTORY {
                return Err(FsError::NotADirectory);
            }
            let name = parts.get(index).ok_or(FsError::InvalidPath)?;
            let (target, _) = self
                .dir_find(&inode, &extents, name)?
                .ok_or(FsError::NotFound)?;
            current = target;
        }
        Ok(current)
    }

    fn dir_find(
        &mut self,
        inode: &Inode,
        extents: &[Extent],
        name: &[u8],
    ) -> Result<Option<(u32, u8)>, FsError> {
        let block_count = ceil_blocks(inode.size);
        for index in 0..block_count {
            let (extent, inner) = extent::locate(extents, index).ok_or(FsError::Corrupt)?;
            let physical = extent.start as u64 + inner;
            let found = self.cache.inspect(&mut self.dev, physical, |data| {
                dir::find(data, name).map(|entry| (entry.inode, entry.kind))
            })?;
            if let Some(pair) = found {
                return Ok(Some(pair));
            }
        }
        Ok(None)
    }

    fn dir_add_entry(
        &mut self,
        dir_inode: &mut Inode,
        dir_extents: &mut Vec<Extent>,
        name: &[u8],
        kind: u8,
        target: u32,
    ) -> Result<(), FsError> {
        let block_count = ceil_blocks(dir_inode.size);
        for index in 0..block_count {
            let (extent, inner) = extent::locate(dir_extents, index).ok_or(FsError::Corrupt)?;
            let physical = extent.start as u64 + inner;
            let placed = self
                .cache
                .mutate(&mut self.dev, physical, |data| dir::add(data, target, kind, name))?;
            if placed {
                return Ok(());
            }
        }
        let run = self.alloc_blocks(1)?;
        let new_block = run[0];
        let mut data = [0u8; BLOCK_BYTES];
        dir::init_empty(&mut data);
        if !dir::add(&mut data, target, kind, name) {
            return Err(FsError::Corrupt);
        }
        dev_write(&mut self.dev, new_block.0 as u64, &data)?;
        if !extent::append(dir_extents, new_block.0, new_block.1) {
            return Err(FsError::Corrupt);
        }
        dir_inode.blocks_used = extent::total_blocks(dir_extents) as u32;
        dir_inode.size += BLOCK_BYTES as u64;
        Ok(())
    }

    fn load_inode(&mut self, inode_no: u32) -> Result<(Inode, Vec<Extent>), FsError> {
        let (table_block, slot_off) = inode_slot_location(&self.geometry, inode_no);
        let (inode, inline) = self
            .cache
            .inspect(&mut self.dev, table_block, |data| {
                inode::decode(&data[slot_off..slot_off + INODE_SIZE].try_into().unwrap())
            })??;
        let mut extents = inline;
        let mut spill = inode.spill_block;
        while spill != 0 {
            let (next, chunk) = self
                .cache
                .inspect(&mut self.dev, spill as u64, |data| extent::decode_spill_block(data))?
                .ok_or(FsError::Corrupt)?;
            extents.extend(chunk);
            spill = next;
        }
        Ok((inode, extents))
    }

    fn store_inode(
        &mut self,
        inode_no: u32,
        inode: &Inode,
        extents: &[Extent],
    ) -> Result<(), FsError> {
        let spill_chain = self.spill_chain_for(inode.spill_block, extents)?;
        let (inline, rest) = extent::split_inline_overflow(extents);
        let mut updated = *inode;
        updated.extent_count = extents.len() as u16;
        updated.spill_block = spill_chain.first().copied().unwrap_or(0);
        let (table_block, slot_off) = inode_slot_location(&self.geometry, inode_no);
        self.cache.mutate(&mut self.dev, table_block, |data| {
            let target: &mut [u8; INODE_SIZE] =
                (&mut data[slot_off..slot_off + INODE_SIZE]).try_into().unwrap();
            inode::encode(&updated, inline, target);
        })?;
        self.write_spill_chain(&spill_chain, rest)
    }

    fn spill_chain_for(
        &mut self,
        head: u32,
        extents: &[Extent],
    ) -> Result<Vec<u32>, FsError> {
        let (_inline, rest) = extent::split_inline_overflow(extents);
        if rest.is_empty() {
            return Ok(Vec::new());
        }
        let chunks = extent::chunk_for_spill(rest);
        let mut chain = self.collect_chain(head)?;
        while chain.len() < chunks.len() {
            let run = self.alloc_blocks(1)?;
            chain.push(run[0].0);
        }
        Ok(chain)
    }

    fn collect_chain(&mut self, head: u32) -> Result<Vec<u32>, FsError> {
        let mut chain = Vec::new();
        let mut next = head;
        while next != 0 {
            chain.push(next);
            let peek = self
                .cache
                .inspect(&mut self.dev, next as u64, |data| {
                    u32::from_le_bytes([data[0], data[1], data[2], data[3]])
                })?;
            next = peek;
        }
        Ok(chain)
    }

    fn write_spill_chain(&mut self, chain: &[u32], rest: &[Extent]) -> Result<(), FsError> {
        let chunks = extent::chunk_for_spill(rest);
        for (index, chunk) in chunks.iter().enumerate() {
            let next = chain.get(index + 1).copied().unwrap_or(0);
            let block_no = chain[index];
            self.cache
                .mutate(&mut self.dev, block_no as u64, |data| {
                    extent::encode_spill_block(data, next, chunk);
                })?;
        }
        Ok(())
    }

    fn alloc_blocks(&mut self, want: u64) -> Result<Vec<(u32, u32)>, FsError> {
        let mut runs = Vec::new();
        let mut remaining = want;
        while remaining > 0 {
            let (start, len) = self
                .block_bitmap
                .allocate_run(self.block_hint, remaining)
                .ok_or(FsError::NoSpace)?;
            self.block_hint = (start + len).min(self.geometry.backup_superblock);
            runs.push((start as u32, len as u32));
            remaining -= len;
        }
        Ok(runs)
    }

    fn zero_blocks(&mut self, runs: &[(u32, u32)]) -> Result<(), FsError> {
        let zero = [0u8; BLOCK_BYTES];
        for (start, len) in runs {
            for offset in 0..*len {
                dev_write(&mut self.dev, *start as u64 + offset as u64, &zero)?;
            }
        }
        Ok(())
    }

    fn alloc_inode(&mut self) -> Result<u32, FsError> {
        let index = self
            .inode_bitmap
            .allocate_one(self.inode_hint)
            .ok_or(FsError::NoInodeSpace)?;
        self.inode_hint = (index + 1).min(self.geometry.inode_count);
        Ok(index as u32)
    }

    fn slot_generation(&mut self, inode_no: u32) -> Result<u32, FsError> {
        let (table_block, slot_off) = inode_slot_location(&self.geometry, inode_no);
        let generation = self.cache.inspect(&mut self.dev, table_block, |data| {
            inode::generation_of(&data[slot_off..slot_off + INODE_SIZE].try_into().unwrap())
        })?;
        Ok(generation)
    }
}
fn stat_of(inode_no: u32, inode: &Inode) -> Stat {
    Stat {
        inode: inode_no,
        generation: inode.generation,
        kind: match inode.kind {
            KIND_DIRECTORY => Kind::Directory,
            _ => Kind::Regular,
        },
        size: inode.size,
        blocks: inode.blocks_used,
        mtime: inode.mtime,
    }
}

fn ceil_blocks(bytes: u64) -> u64 {
    bytes.div_ceil(BLOCK_BYTES as u64)
}

fn inode_slot_location(geometry: &Geometry, inode_no: u32) -> (u64, usize) {
    let slot = inode_no as usize * INODE_SIZE;
    (
        geometry.inode_table_start + (slot / BLOCK_BYTES) as u64,
        slot % BLOCK_BYTES,
    )
}

fn dev_read<D: BlockDevice>(dev: &mut D, block: u64, buf: &mut [u8; BLOCK_BYTES]) -> Result<(), FsError> {
    dev.read_block(block, buf).map_err(FsError::Io)
}

fn dev_write<D: BlockDevice>(dev: &mut D, block: u64, buf: &[u8; BLOCK_BYTES]) -> Result<(), FsError> {
    dev.write_block(block, buf).map_err(FsError::Io)
}

fn read_superblock<D: BlockDevice>(
    dev: &mut D,
    device_blocks: u64,
) -> Result<(Superblock, bool), FsError> {
    let mut block = [0u8; BLOCK_BYTES];
    if dev
        .read_block(SUPERBLOCK_BLOCK, &mut block)
        .is_ok()
    {
        if let Ok(sb) = Superblock::decode(&block) {
            return Ok((sb, false));
        }
    }
    if device_blocks >= 2 {
        dev.read_block(device_blocks - 1, &mut block)
            .map_err(FsError::Io)?;
        if let Ok(sb) = Superblock::decode(&block) {
            return Ok((sb, true));
        }
    }
    Err(FsError::Corrupt)
}

fn write<D: BlockDevice>(dev: &mut D, block: u64, data: &[u8; BLOCK_BYTES]) -> Result<(), FsError> {
    dev.write_block(block, data).map_err(FsError::Io)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::MemoryDevice;
    use crate::spec::DEFAULT_BYTES_PER_INODE;
    use alloc::rc::Rc;
    use alloc::vec;
    use alloc::vec::Vec;
    use core::cell::RefCell;

    const SMALL_BLOCKS: u64 = 1000;
    const EXPECTED_DATA_START: u64 = 25;
    const EXPECTED_BACKUP: u64 = 999;

    #[derive(Clone)]
    struct SharedDisk {
        bytes: Rc<RefCell<Vec<u8>>>,
    }

    impl SharedDisk {
        fn new(blocks: u64) -> SharedDisk {
            SharedDisk {
                bytes: Rc::new(RefCell::new(vec![0u8; blocks as usize * BLOCK_BYTES])),
            }
        }

        fn poke(&mut self, block: u64, data: &[u8; BLOCK_BYTES]) {
            let start = block as usize * BLOCK_BYTES;
            self.bytes.borrow_mut()[start..start + BLOCK_BYTES].copy_from_slice(data);
        }

        fn peek(&self, block: u64) -> [u8; BLOCK_BYTES] {
            let start = block as usize * BLOCK_BYTES;
            let mut out = [0u8; BLOCK_BYTES];
            out.copy_from_slice(&self.bytes.borrow()[start..start + BLOCK_BYTES]);
            out
        }
    }

    impl BlockDevice for SharedDisk {
        fn block_count(&self) -> u64 {
            (self.bytes.borrow().len() / BLOCK_BYTES) as u64
        }

        fn read_block(&mut self, block: u64, buf: &mut [u8]) -> Result<(), crate::error::IoError> {
            let bytes = self.bytes.borrow();
            let start = usize::try_from(block)
                .map_err(|_| crate::error::IoError::Device)?
                .checked_mul(BLOCK_BYTES)
                .ok_or(crate::error::IoError::Device)?;
            let end = start
                .checked_add(BLOCK_BYTES)
                .ok_or(crate::error::IoError::Device)?;
            if end > bytes.len() || buf.len() != BLOCK_BYTES {
                return Err(crate::error::IoError::Device);
            }
            buf.copy_from_slice(&bytes[start..end]);
            Ok(())
        }

        fn write_block(&mut self, block: u64, buf: &[u8]) -> Result<(), crate::error::IoError> {
            let mut bytes = self.bytes.borrow_mut();
            let start = usize::try_from(block)
                .map_err(|_| crate::error::IoError::Device)?
                .checked_mul(BLOCK_BYTES)
                .ok_or(crate::error::IoError::Device)?;
            let end = start
                .checked_add(BLOCK_BYTES)
                .ok_or(crate::error::IoError::Device)?;
            if end > bytes.len() || buf.len() != BLOCK_BYTES {
                return Err(crate::error::IoError::Device);
            }
            bytes[start..end].copy_from_slice(buf);
            Ok(())
        }

        fn flush(&mut self) -> Result<(), crate::error::IoError> {
            Ok(())
        }
    }

    fn default_params<'a>(name: &'a [u8]) -> FormatParams<'a> {
        FormatParams {
            bytes_per_inode: DEFAULT_BYTES_PER_INODE,
            uuid: [7u8; UUID_BYTES],
            volume_name: name,
            created_at: 1000,
        }
    }

    fn format_shared(disk: SharedDisk) -> FileSystem<SharedDisk> {
        FileSystem::format(disk, &default_params(b"testvol")).unwrap()
    }

    #[test]
    fn format_reports_free_counters() {
        let fs = format_shared(SharedDisk::new(SMALL_BLOCKS));
        assert!(!fs.unclean_previous());
        assert_eq!(fs.free_blocks(), 973);
        assert_eq!(fs.free_inodes(), 238);
    }

    #[test]
    fn format_then_unmount_writes_valid_identical_superblocks() {
        let fs = format_shared(SharedDisk::new(SMALL_BLOCKS));
        let disk = fs.unmount().unwrap();
        let primary = Superblock::decode(&disk.peek(SUPERBLOCK_BLOCK)).unwrap();
        let backup = Superblock::decode(&disk.peek(EXPECTED_BACKUP)).unwrap();
        assert_eq!(primary, backup);
        assert_eq!(primary.mount_state, CHECK_CLEAN);
        assert_eq!(primary.txg, 0);
        assert_eq!(primary.total_blocks, SMALL_BLOCKS);
        assert_eq!(primary.free_blocks, 973);
        assert_eq!(primary.free_inodes, 238);
        assert_eq!(primary.uuid, [7u8; UUID_BYTES]);
        assert_eq!(&primary.volume_name[..7], b"testvol");
        assert_eq!(primary.created_at, 1000);
    }

    #[test]
    fn format_initializes_root_inode_on_disk() {
        let fs = format_shared(SharedDisk::new(SMALL_BLOCKS));
        let disk = fs.unmount().unwrap();
        let sb = Superblock::decode(&disk.peek(SUPERBLOCK_BLOCK)).unwrap();
        let table_start = sb.inode_table_start;
        let block = disk.peek(table_start);
        let (root, extents) =
            inode::decode(&block[256..256 + INODE_SIZE].try_into().unwrap()).unwrap();
        assert_eq!(root.kind, KIND_DIRECTORY);
        assert_eq!(root.links, 2);
        assert_eq!(root.size, BLOCK_BYTES as u64);
        assert_eq!(root.blocks_used, 1);
        assert_eq!(root.extent_count, 1);
        assert_eq!(extents[0].start as u64, EXPECTED_DATA_START);
        assert_eq!(extents[0].length, 1);
    }

    #[test]
    fn format_initializes_root_directory_block() {
        let fs = format_shared(SharedDisk::new(SMALL_BLOCKS));
        let disk = fs.unmount().unwrap();
        let block = disk.peek(EXPECTED_DATA_START);
        assert!(dir::is_well_formed(&block));
        let entries = dir::entries(&block);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].name, b".");
        assert_eq!(entries[0].inode, ROOT_INODE);
        assert_eq!(entries[1].name, b"..");
        assert_eq!(entries[1].inode, ROOT_INODE);
    }

    #[test]
    fn format_presets_disk_bitmaps() {
        let fs = format_shared(SharedDisk::new(SMALL_BLOCKS));
        let mut disk = fs.unmount().unwrap();
        let sb = Superblock::decode(&disk.peek(SUPERBLOCK_BLOCK)).unwrap();
        let blocks = BitMap::load(
            &mut disk,
            sb.block_bitmap_start,
            sb.block_bitmap_blocks as u64,
            sb.total_blocks,
        )
        .unwrap();
        assert!(blocks.is_used(0));
        assert!(blocks.is_used(EXPECTED_DATA_START - 1));
        assert!(blocks.is_used(EXPECTED_DATA_START));
        assert!(!blocks.is_used(EXPECTED_DATA_START + 1));
        assert!(blocks.is_used(EXPECTED_BACKUP));
        assert_eq!(blocks.free_count(), 973);

        let inodes = BitMap::load(
            &mut disk,
            sb.inode_bitmap_start,
            sb.inode_bitmap_blocks as u64,
            sb.inode_count as u64,
        )
        .unwrap();
        assert!(inodes.is_used(0) && inodes.is_used(1));
        assert!(!inodes.is_used(2));
        assert_eq!(inodes.free_count(), 238);
    }

    #[test]
    fn mount_marks_unclean_on_disk_until_clean_unmount() {
        let disk = SharedDisk::new(SMALL_BLOCKS);
        let fs = format_shared(disk.clone());
        let disk = fs.unmount().unwrap();

        let first = FileSystem::mount(disk.clone()).unwrap();
        assert!(!first.unclean_previous());
        core::mem::forget(first);
        let peeked = Superblock::decode(&disk.peek(SUPERBLOCK_BLOCK)).unwrap();
        assert_eq!(peeked.mount_state, CHECK_UNCLEAN);

        let second = FileSystem::mount(disk.clone()).unwrap();
        assert!(second.unclean_previous());
        let disk = second.unmount().unwrap();

        let third = FileSystem::mount(disk).unwrap();
        assert!(!third.unclean_previous());
    }

    #[test]
    fn mount_falls_back_to_backup_and_repairs_primary() {
        let disk = SharedDisk::new(SMALL_BLOCKS);
        let fs = format_shared(disk.clone());
        let mut disk = fs.unmount().unwrap();
        disk.poke(SUPERBLOCK_BLOCK, &[0u8; BLOCK_BYTES]);
        let fs = FileSystem::mount(disk.clone()).unwrap();
        assert!(fs.unclean_previous());
        assert_eq!(fs.free_blocks(), 973);
        assert_eq!(fs.free_inodes(), 238);
        let repaired = Superblock::decode(&disk.peek(SUPERBLOCK_BLOCK)).unwrap();
        assert_eq!(repaired.total_blocks, SMALL_BLOCKS);
        assert_eq!(repaired.mount_state, CHECK_UNCLEAN);
    }

    #[test]
    fn mount_corrupt_when_both_superblocks_invalid() {
        let disk = SharedDisk::new(SMALL_BLOCKS);
        let fs = format_shared(disk.clone());
        let mut disk = fs.unmount().unwrap();
        disk.poke(SUPERBLOCK_BLOCK, &[0u8; BLOCK_BYTES]);
        disk.poke(EXPECTED_BACKUP, &[0u8; BLOCK_BYTES]);
        assert_eq!(
            FileSystem::mount(disk.clone()).err(),
            Some(FsError::Corrupt)
        );
    }

    #[test]
    fn mount_rejects_zeroed_device() {
        assert_eq!(
            FileSystem::mount(SharedDisk::new(SMALL_BLOCKS)).err(),
            Some(FsError::Corrupt)
        );
    }

    #[test]
    fn mount_rejects_device_smaller_than_volume() {
        let fs = format_shared(SharedDisk::new(SMALL_BLOCKS));
        let mut big = fs.unmount().unwrap();
        let mut block = [0u8; BLOCK_BYTES];
        big.read_block(SUPERBLOCK_BLOCK, &mut block).unwrap();
        let mut small = MemoryDevice::new(500);
        small.write_block(SUPERBLOCK_BLOCK, &block).unwrap();
        assert_eq!(FileSystem::mount(small).err(), Some(FsError::Corrupt));
    }

    #[test]
    fn mount_tolerates_device_larger_than_volume() {
        let fs = format_shared(SharedDisk::new(SMALL_BLOCKS));
        let source = fs.unmount().unwrap();
        let mut grown = SharedDisk::new(1500);
        for index in 0..SMALL_BLOCKS {
            let bytes = source.peek(index);
            grown.poke(index, &bytes);
        }
        let fs = FileSystem::mount(grown).unwrap();
        assert_eq!(fs.superblock().total_blocks, SMALL_BLOCKS);
        assert_eq!(fs.free_blocks(), 973);
    }

    #[test]
    fn format_rejects_too_small_devices() {
        assert_eq!(
            FileSystem::format(SharedDisk::new(30), &default_params(b"x")).err(),
            Some(FsError::DeviceTooSmall)
        );
        assert_eq!(
            FileSystem::format(SharedDisk::new(0), &default_params(b"x")).err(),
            Some(FsError::DeviceTooSmall)
        );
    }

    #[test]
    fn format_rejects_overlong_volume_name() {
        let long = vec![b'n'; VOLUME_NAME_BYTES + 1];
        assert_eq!(
            FileSystem::format(SharedDisk::new(SMALL_BLOCKS), &default_params(&long)).err(),
            Some(FsError::NameTooLong)
        );
    }

    #[test]
    fn mount_roundtrips_identity_fields() {
        let disk = SharedDisk::new(SMALL_BLOCKS);
        let params = FormatParams {
            bytes_per_inode: DEFAULT_BYTES_PER_INODE,
            uuid: [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16],
            volume_name: b"my-volume",
            created_at: 0xDEAD_BEEF,
        };
        let fs = FileSystem::format(disk, &params).unwrap();
        assert_eq!(fs.superblock().volume_name[..9], *b"my-volume");
        let disk = fs.unmount().unwrap();
        let remounted = FileSystem::mount(disk).unwrap();
        assert_eq!(remounted.superblock().uuid, params.uuid);
        assert_eq!(remounted.superblock().created_at, 0xDEAD_BEEF);
    }

    fn pattern_bytes(len: usize, seed: u8) -> Vec<u8> {
        (0..len).map(|i| (i as u8).wrapping_mul(31).wrapping_add(seed)).collect()
    }

    #[test]
    fn create_write_read_roundtrip() {
        let mut fs = format_shared(SharedDisk::new(SMALL_BLOCKS));
        let h = fs.create(b"/hello").unwrap();
        fs.write(h, 0, b"hello rxfs").unwrap();
        let mut buf = [0u8; 32];
        let n = fs.read(h, 0, &mut buf).unwrap();
        assert_eq!(&buf[..n], b"hello rxfs");
        let st = fs.stat(h).unwrap();
        assert_eq!(st.size, 10);
        assert_eq!(st.kind, Kind::Regular);
        assert_eq!(st.blocks, 1);
    }

    #[test]
    fn create_errors() {
        let mut fs = format_shared(SharedDisk::new(SMALL_BLOCKS));
        assert_eq!(fs.create(b"/missing/file").err(), Some(FsError::NotFound));
        fs.create(b"/dup").unwrap();
        assert_eq!(fs.create(b"/dup").err(), Some(FsError::AlreadyExists));
        assert_eq!(fs.create(b"/").err(), Some(FsError::AlreadyExists));
    }

    #[test]
    fn open_lookup_and_root() {
        let mut fs = format_shared(SharedDisk::new(SMALL_BLOCKS));
        let h = fs.create(b"/thing").unwrap();
        fs.write(h, 0, b"data").unwrap();
        fs.close(h).unwrap();
        let reopened = fs.open(b"/thing").unwrap();
        let mut buf = [0u8; 4];
        assert_eq!(fs.read(reopened, 0, &mut buf).unwrap(), 4);
        assert_eq!(fs.lookup(b"/thing").unwrap().size, 4);
        assert_eq!(fs.open(b"/nope").err(), Some(FsError::NotFound));
        assert_eq!(fs.lookup(b"/nope").err(), Some(FsError::NotFound));
        let root = fs.lookup(b"/").unwrap();
        assert_eq!(root.inode, ROOT_INODE);
        assert_eq!(root.kind, Kind::Directory);
        assert_eq!(root.blocks, 1);
    }

    #[test]
    fn directory_handles_reject_data_io() {
        let mut fs = format_shared(SharedDisk::new(SMALL_BLOCKS));
        let root = fs.open(b"/").unwrap();
        assert_eq!(fs.read(root, 0, &mut [0u8; 4]).err(), Some(FsError::IsADirectory));
        assert_eq!(fs.write(root, 0, b"x").err(), Some(FsError::IsADirectory));
    }

    #[test]
    fn offsets_span_block_boundaries() {
        let mut fs = format_shared(SharedDisk::new(SMALL_BLOCKS));
        let h = fs.create(b"/span").unwrap();
        let data = pattern_bytes(9000, 5);
        fs.write(h, 0, &data).unwrap();
        let mut window = [0u8; 500];
        let n = fs.read(h, 4000, &mut window).unwrap();
        assert_eq!(&window[..n], &data[4000..4500]);
        let n = fs.read(h, 8990, &mut window).unwrap();
        assert_eq!(&window[..n], &data[8990..]);
        assert_eq!(n, 10);
        let n = fs.read(h, 9000, &mut window).unwrap();
        assert_eq!(n, 0);
    }

    #[test]
    fn sparse_gap_reads_zero_filled() {
        let mut fs = format_shared(SharedDisk::new(SMALL_BLOCKS));
        let h = fs.create(b"/gap").unwrap();
        fs.write(h, 10_000, b"tail").unwrap();
        let st = fs.stat(h).unwrap();
        assert_eq!(st.size, 10_004);
        assert_eq!(st.blocks, 3);
        let mut buf = vec![0xFFu8; 10_000];
        fs.read(h, 0, &mut buf).unwrap();
        assert!(buf.iter().all(|&b| b == 0));
        let mut tail = [0u8; 4];
        fs.read(h, 10_000, &mut tail).unwrap();
        assert_eq!(&tail, b"tail");
    }

    #[test]
    fn overwrite_preserves_surroundings() {
        let mut fs = format_shared(SharedDisk::new(SMALL_BLOCKS));
        let h = fs.create(b"/patch").unwrap();
        let data = pattern_bytes(8000, 1);
        fs.write(h, 0, &data).unwrap();
        fs.write(h, 100, b"XXXX").unwrap();
        let mut out = vec![0u8; 8000];
        fs.read(h, 0, &mut out).unwrap();
        assert_eq!(&out[100..104], b"XXXX");
        assert_eq!(&out[..100], &data[..100]);
        assert_eq!(&out[104..], &data[104..]);
    }

    #[test]
    fn sequential_growth_stays_one_extent() {
        let mut fs = format_shared(SharedDisk::new(SMALL_BLOCKS));
        let h = fs.create(b"/seq").unwrap();
        for chunk in 0..40 {
            let data = pattern_bytes(1024, chunk as u8);
            fs.write(h, chunk * 1024, &data).unwrap();
        }
        let st = fs.stat(h).unwrap();
        let (inode, extents) = fs.load_inode(st.inode).unwrap();
        assert_eq!(inode.blocks_used, 10);
        assert_eq!(extent::total_blocks(&extents), 10);
        assert!(extents.len() <= 2);
    }

    #[test]
    fn fragmentation_spills_beyond_inline_capacity() {
        let mut fs = format_shared(SharedDisk::new(SMALL_BLOCKS));
        let f1 = fs.create(b"/f1").unwrap();
        let f2 = fs.create(b"/f2").unwrap();
        let f3 = fs.create(b"/f3").unwrap();
        let mut sizes = [0usize; 3];
        for round in 0..28u8 {
            for (index, handle) in [f1, f2, f3].iter().enumerate() {
                let data = pattern_bytes(8 * BLOCK_BYTES, round * 3 + index as u8);
                fs.write(*handle, sizes[index] as u64, &data).unwrap();
                sizes[index] += data.len();
            }
        }
        let st = fs.stat(f1).unwrap();
        assert_eq!(st.size, sizes[0] as u64);
        let (inode, extents) = fs.load_inode(st.inode).unwrap();
        assert!(inode.spill_block != 0, "expected spill chain");
        assert!(extents.len() > 26);
        let mut whole = vec![0u8; sizes[0]];
        fs.read(f1, 0, &mut whole).unwrap();
        let expected = pattern_bytes(8 * BLOCK_BYTES, 0);
        assert_eq!(&whole[..8 * BLOCK_BYTES], &expected[..]);
        for (index, handle) in [f1, f2, f3].iter().enumerate() {
            let mut end = vec![0u8; 40];
            let n = fs
                .read(*handle, sizes[index] as u64 - 40, &mut end)
                .unwrap();
            let expect = pattern_bytes(8 * BLOCK_BYTES, 27 * 3 + index as u8);
            assert_eq!(&end[..n], &expect[8 * BLOCK_BYTES - 40..]);
        }
    }

    #[test]
    fn allocation_exhaustion_reports_no_space() {
        let disk = SharedDisk::new(47);
        let mut fs = FileSystem::format(disk, &default_params(b"tiny")).unwrap();
        let h = fs.create(b"/hog").unwrap();
        assert_eq!(fs.free_blocks(), 31);
        let big = vec![0u8; 40 * BLOCK_BYTES];
        assert_eq!(fs.write(h, 0, &big).err(), Some(FsError::NoSpace));
    }

    #[test]
    fn inode_exhaustion_reports_no_inode_space() {
        let disk = SharedDisk::new(47);
        let mut fs = FileSystem::format(disk, &default_params(b"tiny")).unwrap();
        let mut handles = Vec::new();
        for i in 0..62 {
            let name = alloc::format!("/f{i}");
            handles.push(fs.create(name.as_bytes()).unwrap());
        }
        assert_eq!(fs.free_inodes(), 0);
        assert_eq!(fs.create(b"/final").err(), Some(FsError::NoInodeSpace));
    }

    #[test]
    fn open_table_capacity_enforced() {
        let mut fs = format_shared(SharedDisk::new(SMALL_BLOCKS));
        let mut handles = Vec::new();
        for i in 0..MAX_OPEN {
            let name = alloc::format!("/open{i}");
            handles.push(fs.create(name.as_bytes()).unwrap());
        }
        assert_eq!(fs.create(b"/overflow").err(), Some(FsError::TooManyOpen));
        assert!(fs.lookup(b"/overflow").is_ok());
        fs.close(handles[0]).unwrap();
        assert!(fs.create(b"/overflow2").is_ok());
    }

    #[test]
    fn stale_handles_are_rejected() {
        let mut fs = format_shared(SharedDisk::new(SMALL_BLOCKS));
        let first = fs.create(b"/gen").unwrap();
        fs.close(first).unwrap();
        assert_eq!(fs.stat(first).err(), Some(FsError::BadHandle));
        assert_eq!(fs.read(first, 0, &mut [0u8; 1]).err(), Some(FsError::BadHandle));
        let second = fs.open(b"/gen").unwrap();
        fs.write(second, 0, b"x").unwrap();
        fs.close(second).unwrap();
        let third = fs.open(b"/gen").unwrap();
        assert_eq!(fs.stat(third).unwrap().size, 1);
    }

    #[test]
    fn recycled_slots_do_not_alias_old_handles() {
        let mut fs = format_shared(SharedDisk::new(SMALL_BLOCKS));
        let mut others = Vec::new();
        for i in 0..8 {
            let name = alloc::format!("/pad{i}");
            others.push(fs.create(name.as_bytes()).unwrap());
        }
        let victim = fs.create(b"/victim").unwrap();
        fs.close(victim).unwrap();
        assert_eq!(fs.stat(victim).err(), Some(FsError::BadHandle));
    }

    #[test]
    fn names_are_case_sensitive() {
        let mut fs = format_shared(SharedDisk::new(SMALL_BLOCKS));
        let upper = fs.create(b"/File").unwrap();
        let lower = fs.create(b"/file").unwrap();
        assert_ne!(fs.stat(upper).unwrap().inode, fs.stat(lower).unwrap().inode);
        fs.write(upper, 0, b"UP").unwrap();
        fs.write(lower, 0, b"low").unwrap();
        let mut buf = [0u8; 8];
        assert_eq!(fs.read(upper, 0, &mut buf).unwrap(), 2);
        assert_eq!(&buf[..2], b"UP");
    }

    #[test]
    fn longest_name_creates_and_resolves() {
        let mut fs = format_shared(SharedDisk::new(SMALL_BLOCKS));
        let mut path = b"/".to_vec();
        path.extend(vec![b'z'; crate::spec::MAX_NAME_BYTES - 1]);
        let h = fs.create(&path).unwrap();
        fs.write(h, 0, b"deep").unwrap();
        fs.close(h).unwrap();
        assert_eq!(fs.lookup(&path).unwrap().size, 4);
    }

    #[test]
    fn written_file_survives_remount() {
        let disk = SharedDisk::new(SMALL_BLOCKS);
        let mut fs = format_shared(disk.clone());
        let h = fs.create(b"/persist").unwrap();
        let data = pattern_bytes(70_000, 9);
        fs.write(h, 0, &data).unwrap();
        fs.close(h).unwrap();
        let mut fs = FileSystem::mount(fs.unmount().unwrap()).unwrap();
        assert!(!fs.unclean_previous());
        let st = fs.lookup(b"/persist").unwrap();
        assert_eq!(st.size, 70_000);
        let h = fs.open(b"/persist").unwrap();
        let mut buf = vec![0u8; 70_000];
        fs.read(h, 0, &mut buf).unwrap();
        assert_eq!(buf, pattern_bytes(70_000, 9));
    }

    #[test]
    fn clean_unmount_remount_roundtrip() {
        let disk = SharedDisk::new(SMALL_BLOCKS);
        let mut fs = format_shared(disk.clone());
        let h = fs.create(b"/clean").unwrap();
        fs.write(h, 0, b"stable").unwrap();
        fs.close(h).unwrap();
        let disk = fs.unmount().unwrap();
        let peeked = Superblock::decode(&disk.peek(SUPERBLOCK_BLOCK)).unwrap();
        assert_eq!(peeked.mount_state, CHECK_CLEAN);
        let mut fs = FileSystem::mount(disk).unwrap();
        assert!(!fs.unclean_previous());
        let h = fs.open(b"/clean").unwrap();
        let mut buf = [0u8; 6];
        fs.read(h, 0, &mut buf).unwrap();
        assert_eq!(&buf, b"stable");
    }

}
