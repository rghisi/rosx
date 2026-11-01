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
    BLOCK_BYTES, Geometry, INODE_SIZE, MAX_DEPTH, MAX_OPEN, RESERVED_PREFIX_BLOCKS, ROOT_INODE,
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
    pending_free_blocks: Vec<(u64, u64)>,
    pending_free_inodes: Vec<u32>,
    sb_dirty: bool,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Report {
    pub leaked_blocks: u64,
    pub leaked_inodes: u64,
    pub dangling_entries: u64,
    pub double_refs: u64,
    pub fixed: bool,
    pub hard_corrupt: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirItem {
    pub inode: u32,
    pub kind: Kind,
    pub name: Vec<u8>,
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
        sb.mount_state = CHECK_UNCLEAN;

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
            pending_free_blocks: Vec::new(),
            pending_free_inodes: Vec::new(),
            sb_dirty: false,
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
            pending_free_blocks: Vec::new(),
            pending_free_inodes: Vec::new(),
            sb_dirty: false,
        })
    }

    pub fn sync(&mut self) -> Result<(), FsError> {
        self.commit(false)
    }

    pub fn unmount(mut self) -> Result<D, FsError> {
        self.commit(true)?;
        Ok(self.dev)
    }

    fn commit(&mut self, clean: bool) -> Result<(), FsError> {
        let has_frees =
            !self.pending_free_blocks.is_empty() || !self.pending_free_inodes.is_empty();
        let work = self.cache.any_dirty() || has_frees;
        if work {
            if self.cache.any_dirty() {
                self.store_bitmaps()?;
                self.cache.flush(&mut self.dev)?;
            }
            if has_frees {
                for (start, len) in self.pending_free_blocks.drain(..) {
                    self.block_bitmap.free_range(start, len);
                }
                for inode_no in self.pending_free_inodes.drain(..) {
                    self.inode_bitmap.set_free(inode_no as u64);
                }
                self.store_bitmaps()?;
            }
        }
        let state = if clean { CHECK_CLEAN } else { CHECK_UNCLEAN };
        if !work && !self.sb_dirty && self.sb.mount_state == state {
            return Ok(());
        }
        if work {
            self.sb.txg += 1;
        }
        self.sb.mount_state = state;
        self.sb.free_blocks = self.block_bitmap.free_count();
        self.sb.free_inodes = self.inode_bitmap.free_count() as u32;
        self.sb.alloc_hint = self.block_hint as u32;
        let mut sb_block = [0u8; BLOCK_BYTES];
        self.sb.encode(&mut sb_block);
        write(&mut self.dev, SUPERBLOCK_BLOCK, &sb_block)?;
        write(&mut self.dev, self.geometry.backup_superblock, &sb_block)?;
        self.dev.flush().map_err(FsError::Io)?;
        self.sb_dirty = false;
        Ok(())
    }

    fn store_bitmaps(&mut self) -> Result<(), FsError> {
        self.block_bitmap
            .store(&mut self.dev, self.geometry.block_bitmap_start)?;
        self.inode_bitmap
            .store(&mut self.dev, self.geometry.inode_bitmap_start)?;
        Ok(())
    }

    pub fn superblock(&self) -> &Superblock {
        &self.sb
    }

    pub fn unclean_previous(&self) -> bool {
        self.unclean_previous
    }

    pub fn free_blocks(&self) -> u64 {
        self.block_bitmap.free_count()
            + self.pending_free_blocks.iter().map(|(_, len)| *len).sum::<u64>()
    }

    pub fn free_inodes(&self) -> u64 {
        self.inode_bitmap.free_count() + self.pending_free_inodes.len() as u64
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
        if inode.kind == KIND_FREE {
            return Err(FsError::BadHandle);
        }
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
        if inode.kind == KIND_FREE {
            return Err(FsError::BadHandle);
        }
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
        if inode.kind == KIND_FREE {
            return Err(FsError::BadHandle);
        }
        Ok(stat_of(inode_no, &inode))
    }

    pub fn close(&mut self, handle: Handle) -> Result<(), FsError> {
        self.validate_handle(handle)?;
        self.opens[handle.slot as usize] = None;
        Ok(())
    }

    pub fn mkdir(&mut self, path_bytes: &[u8]) -> Result<(), FsError> {
        let parts = path::parse(path_bytes)?;
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
        let dir_no = self.alloc_inode()?;
        let generation = self.slot_generation(dir_no)?;
        let runs = self.alloc_blocks(1)?;
        let first_block = runs[0];
        let mut data = [0u8; BLOCK_BYTES];
        dir::init_dot_entries(&mut data, dir_no, parent_no);
        dev_write(&mut self.dev, first_block.0 as u64, &data)?;
        let mut dir_extents = Vec::new();
        extent::append(&mut dir_extents, first_block.0, first_block.1);
        let mut inode = Inode::new(KIND_DIRECTORY, MODE_DIRECTORY, generation);
        inode.size = BLOCK_BYTES as u64;
        inode.blocks_used = extent::total_blocks(&dir_extents) as u32;
        inode.mtime = self.sb.txg;
        self.store_inode(dir_no, &inode, &dir_extents)?;
        self.dir_add_entry(&mut parent_inode, &mut parent_extents, name, KIND_DIRECTORY, dir_no)?;
        parent_inode.mtime = self.sb.txg;
        self.store_inode(parent_no, &parent_inode, &parent_extents)?;
        Ok(())
    }

    pub fn remove(&mut self, path_bytes: &[u8]) -> Result<(), FsError> {
        let parts = path::parse(path_bytes)?;
        if parts.is_root() {
            return Err(FsError::InvalidPath);
        }
        let parent_no = self.resolve_prefix(&parts, parts.len() - 1)?;
        let name = parts.get(parts.len() - 1).ok_or(FsError::InvalidPath)?;
        let (mut parent_inode, parent_extents) = self.load_inode(parent_no)?;
        if parent_inode.kind != KIND_DIRECTORY {
            return Err(FsError::NotADirectory);
        }
        let (target, kind) = self
            .dir_find(&parent_inode, &parent_extents, name)?
            .ok_or(FsError::NotFound)?;
        if kind == KIND_DIRECTORY {
            return Err(FsError::IsADirectory);
        }
        self.dir_remove_entry(&parent_inode, &parent_extents, name)?;
        parent_inode.mtime = self.sb.txg;
        self.store_inode(parent_no, &parent_inode, &parent_extents)?;
        self.release_inode(target)
    }

    pub fn rmdir(&mut self, path_bytes: &[u8]) -> Result<(), FsError> {
        let parts = path::parse(path_bytes)?;
        if parts.is_root() {
            return Err(FsError::InvalidPath);
        }
        let parent_no = self.resolve_prefix(&parts, parts.len() - 1)?;
        let name = parts.get(parts.len() - 1).ok_or(FsError::InvalidPath)?;
        let (mut parent_inode, parent_extents) = self.load_inode(parent_no)?;
        if parent_inode.kind != KIND_DIRECTORY {
            return Err(FsError::NotADirectory);
        }
        let (target, kind) = self
            .dir_find(&parent_inode, &parent_extents, name)?
            .ok_or(FsError::NotFound)?;
        if kind != KIND_DIRECTORY {
            return Err(FsError::NotADirectory);
        }
        if self.dir_live_entries(target)? != 2 {
            return Err(FsError::DirectoryNotEmpty);
        }
        self.dir_remove_entry(&parent_inode, &parent_extents, name)?;
        parent_inode.mtime = self.sb.txg;
        self.store_inode(parent_no, &parent_inode, &parent_extents)?;
        self.release_inode(target)
    }

    pub fn readdir(&mut self, path_bytes: &[u8]) -> Result<Vec<DirItem>, FsError> {
        let parts = path::parse(path_bytes)?;
        let dir_no = self.resolve(&parts)?;
        let (inode, extents) = self.load_inode(dir_no)?;
        if inode.kind != KIND_DIRECTORY {
            return Err(FsError::NotADirectory);
        }
        let mut items = Vec::new();
        for index in 0..ceil_blocks(inode.size) {
            let (extent, inner) = extent::locate(&extents, index).ok_or(FsError::Corrupt)?;
            let physical = extent.start as u64 + inner;
            let mut found = self.cache.inspect(&mut self.dev, physical, |data| {
                dir::entries(data)
                    .iter()
                    .map(|entry| (entry.inode, entry.kind, entry.name.to_vec()))
                    .collect::<Vec<(u32, u8, Vec<u8>)>>()
            })?;
            items.extend(found.drain(..).map(|(ino, kind, name)| DirItem {
                inode: ino,
                kind: kind_to_kind(kind),
                name,
            }));
        }
        Ok(items)
    }

    fn dir_live_entries(&mut self, dir_no: u32) -> Result<usize, FsError> {
        let (inode, extents) = self.load_inode(dir_no)?;
        let mut count = 0usize;
        for index in 0..ceil_blocks(inode.size) {
            let (extent, inner) = extent::locate(&extents, index).ok_or(FsError::Corrupt)?;
            let physical = extent.start as u64 + inner;
            count += self
                .cache
                .inspect(&mut self.dev, physical, |data| dir::live_count(data))?;
        }
        Ok(count)
    }

    fn dir_remove_entry(
        &mut self,
        dir_inode: &Inode,
        dir_extents: &[Extent],
        name: &[u8],
    ) -> Result<(), FsError> {
        for index in 0..ceil_blocks(dir_inode.size) {
            let (extent, inner) = extent::locate(dir_extents, index).ok_or(FsError::Corrupt)?;
            let physical = extent.start as u64 + inner;
            let removed = self
                .cache
                .mutate(&mut self.dev, physical, |data| dir::remove(data, name))?;
            if removed {
                return Ok(());
            }
        }
        Err(FsError::NotFound)
    }

    fn release_inode(&mut self, inode_no: u32) -> Result<(), FsError> {
        let (inode, extents) = self.load_inode(inode_no)?;
        for extent in &extents {
            self.pending_free_blocks
                .push((extent.start as u64, extent.length as u64));
            self.cache.invalidate(extent.start as u64);
        }
        let mut next = inode.spill_block;
        while next != 0 {
            let peek = self
                .cache
                .inspect(&mut self.dev, next as u64, |data| {
                    u32::from_le_bytes([data[0], data[1], data[2], data[3]])
                })?;
            self.pending_free_blocks.push((next as u64, 1));
            self.cache.invalidate(next as u64);
            next = peek;
        }
        let freed = Inode::free(inode.generation.wrapping_add(1));
        let (table_block, slot_off) = inode_slot_location(&self.geometry, inode_no);
        self.cache.mutate(&mut self.dev, table_block, |data| {
            let target: &mut [u8; INODE_SIZE] =
                (&mut data[slot_off..slot_off + INODE_SIZE]).try_into().unwrap();
            inode::encode(&freed, &[], target);
        })?;
        self.pending_free_inodes.push(inode_no);
        Ok(())
    }

    pub fn check(&mut self) -> Result<Report, FsError> {
        self.commit(false)?;
        let total_blocks = self.geometry.total_blocks;
        let inode_count = self.geometry.inode_count;
        let mut checker = Checker {
            fs: self,
            ref_blocks: BitMap::empty(total_blocks),
            ref_inodes: BitMap::empty(inode_count),
            visited: BitMap::empty(inode_count),
            report: Report::default(),
        };
        checker.run()?;
        checker.finish()?;
        Ok(checker.report)
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
struct Checker<'a, D: BlockDevice> {
    fs: &'a mut FileSystem<D>,
    ref_blocks: BitMap,
    ref_inodes: BitMap,
    visited: BitMap,
    report: Report,
}

impl<D: BlockDevice> Checker<'_, D> {
    fn run(&mut self) -> Result<(), FsError> {
        let root = match self.fs.load_inode(ROOT_INODE) {
            Ok(pair) if pair.0.kind == KIND_DIRECTORY => pair,
            _ => {
                self.report.hard_corrupt = true;
                return Ok(());
            }
        };
        self.ref_inodes.set_used(ROOT_INODE as u64);
        self.mark_extents(ROOT_INODE, &root.0, &root.1)?;
        let mut stack = Vec::new();
        stack.push((ROOT_INODE, ROOT_INODE, 0usize));
        while let Some((dir_no, parent, depth)) = stack.pop() {
            if self.visited.is_used(u64::from(dir_no)) {
                self.report.hard_corrupt = true;
                continue;
            }
            self.visited.set_used(u64::from(dir_no));
            self.walk_dir(dir_no, parent, depth, &mut stack)?;
        }
        Ok(())
    }

    fn walk_dir(
        &mut self,
        dir_no: u32,
        parent: u32,
        depth: usize,
        stack: &mut Vec<(u32, u32, usize)>,
    ) -> Result<(), FsError> {
        let (dir_inode, dir_extents) = self.fs.load_inode(dir_no)?;
        if dir_inode.kind != KIND_DIRECTORY {
            self.report.hard_corrupt = true;
            return Ok(());
        }
        for index in 0..ceil_blocks(dir_inode.size) {
            let located = extent::locate(&dir_extents, index);
            let Some((extent, inner)) = located else {
                self.report.hard_corrupt = true;
                continue;
            };
            let physical = extent.start as u64 + inner;
            let well_formed = self
                .fs
                .cache
                .inspect(&mut self.fs.dev, physical, |data| dir::is_well_formed(data))?;
            if !well_formed {
                self.report.hard_corrupt = true;
                continue;
            }
            if index == 0 {
                self.fix_dot_entries(physical, dir_no, parent)?;
            }
            let entries = self
                .fs
                .cache
                .inspect(&mut self.fs.dev, physical, |data| {
                    dir::entries(data)
                        .iter()
                        .filter(|entry| entry.name != b"." && entry.name != b"..")
                        .map(|entry| (entry.name.to_vec(), entry.inode, entry.kind))
                        .collect::<Vec<(Vec<u8>, u32, u8)>>()
                })?;
            for (name, target, entry_kind) in entries {
                self.visit_entry(
                    dir_no,
                    depth,
                    &dir_inode,
                    &dir_extents,
                    &name,
                    target,
                    entry_kind,
                    stack,
                )?;
            }
        }
        Ok(())
    }

    fn fix_dot_entries(&mut self, physical: u64, dir_no: u32, parent: u32) -> Result<(), FsError> {
        let (dot, dot_dot) = self
            .fs
            .cache
            .inspect(&mut self.fs.dev, physical, |data| {
                (
                    dir::find(data, b".").map(|entry| (entry.inode, entry.kind)),
                    dir::find(data, b"..").map(|entry| (entry.inode, entry.kind)),
                )
            })?;
        if dot.is_none() || dot_dot.is_none() {
            self.report.hard_corrupt = true;
        }
        if dot.is_some_and(|found| found != (dir_no, KIND_DIRECTORY)) {
            self.fs.cache.mutate(&mut self.fs.dev, physical, |data| {
                dir::patch(data, b".", dir_no, KIND_DIRECTORY)
            })?;
            self.report.fixed = true;
        }
        if dot_dot.is_some_and(|found| found != (parent, KIND_DIRECTORY)) {
            self.fs.cache.mutate(&mut self.fs.dev, physical, |data| {
                dir::patch(data, b"..", parent, KIND_DIRECTORY)
            })?;
            self.report.fixed = true;
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn visit_entry(
        &mut self,
        dir_no: u32,
        depth: usize,
        dir_inode: &Inode,
        dir_extents: &[Extent],
        name: &[u8],
        target: u32,
        entry_kind: u8,
        stack: &mut Vec<(u32, u32, usize)>,
    ) -> Result<(), FsError> {
        let loaded = if target == 0 {
            None
        } else {
            self.fs.load_inode(target).ok()
        };
        let (inode, extents) = match loaded {
            Some(pair) if pair.0.kind != KIND_FREE => pair,
            _ => {
                if self.repair_dir_entry(dir_inode, dir_extents, name, true, 0, 0)? {
                    self.report.dangling_entries += 1;
                    self.report.fixed = true;
                }
                return Ok(());
            }
        };
        if entry_kind != inode.kind {
            if self.repair_dir_entry(dir_inode, dir_extents, name, false, target, inode.kind)? {
                self.report.fixed = true;
            }
        }
        if self.ref_inodes.is_used(u64::from(target)) {
            return Ok(());
        }
        self.ref_inodes.set_used(u64::from(target));
        self.mark_extents(target, &inode, &extents)?;
        if inode.kind == KIND_DIRECTORY {
            if depth + 1 > MAX_DEPTH {
                self.report.hard_corrupt = true;
            } else {
                stack.push((target, dir_no, depth + 1));
            }
        }
        Ok(())
    }

    fn repair_dir_entry(
        &mut self,
        dir_inode: &Inode,
        dir_extents: &[Extent],
        name: &[u8],
        remove: bool,
        inode: u32,
        kind: u8,
    ) -> Result<bool, FsError> {
        for index in 0..ceil_blocks(dir_inode.size) {
            let Some((extent, inner)) = extent::locate(dir_extents, index) else {
                return Ok(false);
            };
            let physical = extent.start as u64 + inner;
            let applied = if remove {
                self.fs
                    .cache
                    .mutate(&mut self.fs.dev, physical, |data| dir::remove(data, name))?
            } else {
                self.fs.cache.mutate(&mut self.fs.dev, physical, |data| {
                    dir::patch(data, name, inode, kind)
                })?
            };
            if applied {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn mark_extents(&mut self, no: u32, inode: &Inode, extents: &[Extent]) -> Result<(), FsError> {
        let mut rebuilt: Vec<Extent> = Vec::new();
        let mut changed = false;
        for extent in extents {
            let start = extent.start as u64;
            if start < self.fs.geometry.data_start
                || extent.end() > self.fs.geometry.backup_superblock
            {
                self.report.hard_corrupt = true;
                changed = true;
                continue;
            }
            for block in start..extent.end() {
                if self.ref_blocks.is_used(block) {
                    self.report.double_refs += 1;
                    changed = true;
                } else {
                    self.ref_blocks.set_used(block);
                    extent::append(&mut rebuilt, block as u32, 1);
                }
            }
        }
        if changed {
            let blocks = extent::total_blocks(&rebuilt);
            let mut repaired = *inode;
            repaired.blocks_used = blocks as u32;
            repaired.size = if inode.kind == KIND_DIRECTORY {
                blocks * BLOCK_BYTES as u64
            } else {
                inode.size.min(blocks * BLOCK_BYTES as u64)
            };
            self.fs.store_inode(no, &repaired, &rebuilt)?;
            self.report.fixed = true;
        } else if inode.kind != KIND_DIRECTORY {
            let cap = inode.blocks_used as u64 * BLOCK_BYTES as u64;
            if inode.size > cap {
                let mut repaired = *inode;
                repaired.size = cap;
                self.fs.store_inode(no, &repaired, extents)?;
                self.report.fixed = true;
            }
        }
        self.mark_spill(inode.spill_block)
    }

    fn mark_spill(&mut self, head: u32) -> Result<(), FsError> {
        let mut next = head;
        let mut hops = 0u64;
        while next != 0 {
            let block = u64::from(next);
            if block < self.fs.geometry.data_start
                || block >= self.fs.geometry.backup_superblock
                || self.ref_blocks.is_used(block)
            {
                self.report.hard_corrupt = true;
                break;
            }
            self.ref_blocks.set_used(block);
            let peek = self.fs.cache.inspect(&mut self.fs.dev, block, |data| {
                u32::from_le_bytes([data[0], data[1], data[2], data[3]])
            })?;
            next = peek;
            hops += 1;
            if hops > self.fs.geometry.data_blocks {
                self.report.hard_corrupt = true;
                break;
            }
        }
        Ok(())
    }

    fn finish(&mut self) -> Result<(), FsError> {
        for index in 2..self.fs.geometry.inode_count {
            let no = index as u32;
            let referenced = self.ref_inodes.is_used(index);
            let allocated = self.fs.inode_bitmap.is_used(index);
            if allocated && !referenced {
                let generation = self.fs.slot_generation(no)?;
                let freed = Inode::free(generation.wrapping_add(1));
                let (table_block, slot_off) = inode_slot_location(&self.fs.geometry, no);
                self.fs.cache.mutate(&mut self.fs.dev, table_block, |data| {
                    let target: &mut [u8; INODE_SIZE] =
                        (&mut data[slot_off..slot_off + INODE_SIZE]).try_into().unwrap();
                    inode::encode(&freed, &[], target);
                })?;
                self.fs.inode_bitmap.set_free(index);
                self.report.leaked_inodes += 1;
                self.report.fixed = true;
            } else if referenced && !allocated {
                self.fs.inode_bitmap.set_used(index);
                self.report.fixed = true;
            }
        }
        for block in self.fs.geometry.data_start..self.fs.geometry.backup_superblock {
            let allocated = self.fs.block_bitmap.is_used(block);
            let referenced = self.ref_blocks.is_used(block);
            if allocated && !referenced {
                self.fs.block_bitmap.set_free(block);
                self.report.leaked_blocks += 1;
                self.report.fixed = true;
            } else if referenced && !allocated {
                self.fs.block_bitmap.set_used(block);
                self.report.fixed = true;
            }
        }
        if self.report.fixed {
            self.fs.store_bitmaps()?;
            self.fs.sb_dirty = true;
            self.fs.commit(false)?;
        }
        Ok(())
    }
}

fn kind_to_kind(raw: u8) -> Kind {
    match raw {
        KIND_DIRECTORY => Kind::Directory,
        _ => Kind::Regular,
    }
}

fn stat_of(inode_no: u32, inode: &Inode) -> Stat {
    Stat {
        inode: inode_no,
        generation: inode.generation,
        kind: kind_to_kind(inode.kind),
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
    use alloc::collections::{BTreeMap, BTreeSet};
    use core::cell::Cell;
    use core::cell::RefCell;

    const SMALL_BLOCKS: u64 = 1000;
    const EXPECTED_DATA_START: u64 = 25;
    const EXPECTED_BACKUP: u64 = 999;

    #[derive(Clone)]
    struct SharedDisk {
        bytes: Rc<RefCell<Vec<u8>>>,
        writes: Rc<RefCell<usize>>,
    }

    impl SharedDisk {
        fn new(blocks: u64) -> SharedDisk {
            SharedDisk {
                bytes: Rc::new(RefCell::new(vec![0u8; blocks as usize * BLOCK_BYTES])),
                writes: Rc::new(RefCell::new(0)),
            }
        }

        fn write_count(&self) -> usize {
            *self.writes.borrow()
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
            *self.writes.borrow_mut() += 1;
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


    #[test]
    fn mkdir_create_and_read_nested() {
        let mut fs = format_shared(SharedDisk::new(SMALL_BLOCKS));
        fs.mkdir(b"/a").unwrap();
        let h = fs.create(b"/a/f").unwrap();
        fs.write(h, 0, b"nested").unwrap();
        fs.close(h).unwrap();
        let mut buf = [0u8; 6];
        let h = fs.open(b"/a/f").unwrap();
        assert_eq!(fs.read(h, 0, &mut buf).unwrap(), 6);
        assert_eq!(&buf, b"nested");
    }

    #[test]
    fn mkdir_errors() {
        let mut fs = format_shared(SharedDisk::new(SMALL_BLOCKS));
        assert_eq!(fs.mkdir(b"/").err(), Some(FsError::AlreadyExists));
        fs.mkdir(b"/dup").unwrap();
        assert_eq!(fs.mkdir(b"/dup").err(), Some(FsError::AlreadyExists));
        assert_eq!(fs.mkdir(b"/missing/deep").err(), Some(FsError::NotFound));
        let file = fs.create(b"/file").unwrap();
        assert_eq!(fs.mkdir(b"/file/inside").err(), Some(FsError::NotADirectory));
        let _ = file;
    }

    #[test]
    fn mkdir_updates_kind_and_stat() {
        let mut fs = format_shared(SharedDisk::new(SMALL_BLOCKS));
        fs.mkdir(b"/sub").unwrap();
        let st = fs.lookup(b"/sub").unwrap();
        assert_eq!(st.kind, Kind::Directory);
        assert_eq!(st.size, BLOCK_BYTES as u64);
        assert_eq!(st.blocks, 1);
    }

    #[test]
    fn deep_tree_resolution() {
        let mut fs = format_shared(SharedDisk::new(SMALL_BLOCKS));
        fs.mkdir(b"/a").unwrap();
        fs.mkdir(b"/a/b").unwrap();
        fs.mkdir(b"/a/b/c/").unwrap();
        let h = fs.create(b"/a/b/c/deep.txt").unwrap();
        fs.write(h, 0, b"bottom").unwrap();
        assert_eq!(fs.lookup(b"/a/b/c/deep.txt").unwrap().size, 6);
    }

    #[test]
    fn readdir_lists_dots_and_entries_in_order() {
        let mut fs = format_shared(SharedDisk::new(SMALL_BLOCKS));
        fs.mkdir(b"/sub").unwrap();
        fs.create(b"/one").unwrap();
        let items = fs.readdir(b"/").unwrap();
        let names: Vec<&[u8]> = items.iter().map(|item| item.name.as_slice()).collect();
        assert_eq!(names, vec![&b"."[..], &b".."[..], &b"sub"[..], &b"one"[..]]);
        assert_eq!(items[2].kind, Kind::Directory);
        assert_eq!(items[3].kind, Kind::Regular);
        assert_eq!(items[0].inode, ROOT_INODE);
    }

    #[test]
    fn readdir_requires_directory() {
        let mut fs = format_shared(SharedDisk::new(SMALL_BLOCKS));
        fs.create(b"/plain").unwrap();
        assert_eq!(fs.readdir(b"/plain").err(), Some(FsError::NotADirectory));
        assert_eq!(fs.readdir(b"/ghost").err(), Some(FsError::NotFound));
    }

    #[test]
    fn remove_frees_blocks_and_entry() {
        let mut fs = format_shared(SharedDisk::new(SMALL_BLOCKS));
        let before = fs.free_blocks();
        let before_inodes = fs.free_inodes();
        let h = fs.create(b"/gone").unwrap();
        fs.write(h, 0, &vec![0xABu8; 10 * BLOCK_BYTES]).unwrap();
        fs.close(h).unwrap();
        assert_eq!(fs.free_blocks(), before - 10);
        fs.remove(b"/gone").unwrap();
        assert_eq!(fs.free_blocks(), before);
        assert_eq!(fs.free_inodes(), before_inodes);
        assert_eq!(fs.lookup(b"/gone").err(), Some(FsError::NotFound));
        assert_eq!(fs.readdir(b"/").unwrap().len(), 2);
    }

    #[test]
    fn remove_errors() {
        let mut fs = format_shared(SharedDisk::new(SMALL_BLOCKS));
        assert_eq!(fs.remove(b"/nope").err(), Some(FsError::NotFound));
        assert_eq!(fs.remove(b"/").err(), Some(FsError::InvalidPath));
        fs.mkdir(b"/adir").unwrap();
        assert_eq!(fs.remove(b"/adir").err(), Some(FsError::IsADirectory));
    }

    #[test]
    fn rmdir_roundtrip_frees_everything() {
        let mut fs = format_shared(SharedDisk::new(SMALL_BLOCKS));
        let before = fs.free_blocks();
        let before_inodes = fs.free_inodes();
        fs.mkdir(b"/box").unwrap();
        let inner = fs.create(b"/box/thing").unwrap();
        fs.write(inner, 0, b"data").unwrap();
        fs.close(inner).unwrap();
        assert_eq!(fs.rmdir(b"/box").err(), Some(FsError::DirectoryNotEmpty));
        fs.remove(b"/box/thing").unwrap();
        fs.rmdir(b"/box").unwrap();
        assert_eq!(fs.free_blocks(), before);
        assert_eq!(fs.free_inodes(), before_inodes);
        assert_eq!(fs.lookup(b"/box").err(), Some(FsError::NotFound));
    }

    #[test]
    fn rmdir_errors() {
        let mut fs = format_shared(SharedDisk::new(SMALL_BLOCKS));
        assert_eq!(fs.rmdir(b"/").err(), Some(FsError::InvalidPath));
        assert_eq!(fs.rmdir(b"/missing").err(), Some(FsError::NotFound));
        fs.create(b"/plain").unwrap();
        assert_eq!(fs.rmdir(b"/plain").err(), Some(FsError::NotADirectory));
    }

    #[test]
    fn remove_bumps_generation_on_freed_slot() {
        let mut fs = format_shared(SharedDisk::new(SMALL_BLOCKS));
        let handle = fs.create(b"/cycle").unwrap();
        let st = fs.stat(handle).unwrap();
        fs.close(handle).unwrap();
        fs.remove(b"/cycle").unwrap();
        assert_eq!(fs.slot_generation(st.inode).unwrap(), st.generation + 1);
    }

    #[test]
    fn open_handle_on_removed_file_is_bad_handle() {
        let mut fs = format_shared(SharedDisk::new(SMALL_BLOCKS));
        let h = fs.create(b"/doomed").unwrap();
        fs.write(h, 0, b"data").unwrap();
        fs.remove(b"/doomed").unwrap();
        assert_eq!(fs.stat(h).err(), Some(FsError::BadHandle));
        assert_eq!(fs.read(h, 0, &mut [0u8; 1]).err(), Some(FsError::BadHandle));
        assert_eq!(fs.write(h, 0, b"x").err(), Some(FsError::BadHandle));
    }

    #[test]
    fn removed_spill_file_frees_chain_blocks() {
        let mut fs = format_shared(SharedDisk::new(SMALL_BLOCKS));
        let f1 = fs.create(b"/sp1").unwrap();
        let f2 = fs.create(b"/sp2").unwrap();
        let mut f1_size = 0usize;
        let mut f2_size = 0usize;
        for round in 0..30usize {
            let data = pattern_bytes(8 * BLOCK_BYTES, round as u8);
            fs.write(f1, f1_size as u64, &data).unwrap();
            f1_size += data.len();
            let other = pattern_bytes(4 * BLOCK_BYTES, (round + 100) as u8);
            fs.write(f2, f2_size as u64, &other).unwrap();
            f2_size += other.len();
        }
        let f1_stat = fs.stat(f1).unwrap();
        assert_ne!(fs.load_inode(f1_stat.inode).unwrap().0.spill_block, 0);
        let blocks_before = fs.free_blocks();
        fs.close(f1).unwrap();
        fs.remove(b"/sp1").unwrap();
        assert_eq!(fs.free_blocks(), blocks_before + u64::from(f1_stat.blocks) + 1);
        let mut tail = vec![0u8; 40];
        let n = fs.read(f2, (f2_size - 40) as u64, &mut tail).unwrap();
        let expect = pattern_bytes(4 * BLOCK_BYTES, 129);
        assert_eq!(&tail[..n], &expect[4 * BLOCK_BYTES - 40..]);
        fs.close(f2).unwrap();
        let mut fs = FileSystem::mount(fs.unmount().unwrap()).unwrap();
        assert_eq!(fs.lookup(b"/sp1").err(), Some(FsError::NotFound));
        assert_eq!(fs.lookup(b"/sp2").unwrap().size, f2_size as u64);
    }

    #[test]
    fn sync_makes_writes_survive_power_loss() {
        let disk = SharedDisk::new(SMALL_BLOCKS);
        let mut fs = format_shared(disk.clone());
        let h = fs.create(b"/durable").unwrap();
        let data = pattern_bytes(5_000, 3);
        fs.write(h, 0, &data).unwrap();
        fs.sync().unwrap();
        core::mem::forget(fs);
        let mut fs = FileSystem::mount(disk).unwrap();
        assert!(fs.unclean_previous());
        let st = fs.lookup(b"/durable").unwrap();
        assert_eq!(st.size, 5_000);
        let h = fs.open(b"/durable").unwrap();
        let mut buf = vec![0u8; 5_000];
        fs.read(h, 0, &mut buf).unwrap();
        assert_eq!(buf, data);
    }

    #[test]
    fn no_op_sync_touches_no_disk() {
        let mut fs = format_shared(SharedDisk::new(SMALL_BLOCKS));
        let quiet = fs.dev.write_count();
        fs.sync().unwrap();
        fs.sync().unwrap();
        assert_eq!(fs.dev.write_count(), quiet);
        fs.create(b"/x").unwrap();
        fs.sync().unwrap();
        assert!(fs.dev.write_count() > quiet);
        let after_work = fs.dev.write_count();
        fs.sync().unwrap();
        assert_eq!(fs.dev.write_count(), after_work);
    }

    #[test]
    fn sync_bumps_txg_only_on_work() {
        let mut fs = format_shared(SharedDisk::new(SMALL_BLOCKS));
        assert_eq!(fs.superblock().txg, 0);
        fs.sync().unwrap();
        assert_eq!(fs.superblock().txg, 0);
        fs.mkdir(b"/t").unwrap();
        fs.sync().unwrap();
        assert_eq!(fs.superblock().txg, 1);
        fs.sync().unwrap();
        assert_eq!(fs.superblock().txg, 1);
    }

    #[test]
    fn unsynced_remove_never_overwrites_durable_file() {
        let disk = SharedDisk::new(SMALL_BLOCKS);
        let mut fs = format_shared(disk.clone());
        let f = fs.create(b"/keeper").unwrap();
        let keep = pattern_bytes(8 * BLOCK_BYTES, 42);
        fs.write(f, 0, &keep).unwrap();
        fs.close(f).unwrap();
        fs.sync().unwrap();
        fs.remove(b"/keeper").unwrap();
        let g = fs.create(b"/squatter").unwrap();
        fs.write(g, 0, &vec![0x5Au8; 8 * BLOCK_BYTES]).unwrap();
        fs.close(g).unwrap();
        core::mem::forget(fs);
        let mut fs = FileSystem::mount(disk).unwrap();
        let st = fs.lookup(b"/keeper").unwrap();
        assert_eq!(st.size, keep.len() as u64);
        let h = fs.open(b"/keeper").unwrap();
        let mut buf = vec![0u8; keep.len()];
        fs.read(h, 0, &mut buf).unwrap();
        assert_eq!(buf, keep);
        assert_eq!(fs.lookup(b"/squatter").err(), Some(FsError::NotFound));
    }

    #[test]
    fn remove_committed_by_sync_survives() {
        let disk = SharedDisk::new(SMALL_BLOCKS);
        let mut fs = format_shared(disk.clone());
        let h = fs.create(b"/temporary").unwrap();
        fs.write(h, 0, &vec![1u8; 4 * BLOCK_BYTES]).unwrap();
        fs.close(h).unwrap();
        fs.remove(b"/temporary").unwrap();
        fs.sync().unwrap();
        core::mem::forget(fs);
        let mut fs = FileSystem::mount(disk).unwrap();
        assert_eq!(fs.lookup(b"/temporary").err(), Some(FsError::NotFound));
        assert_eq!(fs.free_blocks(), 973);
        assert_eq!(fs.free_inodes(), 238);
    }

    const TEST_BLOCK_BITMAP: u64 = 8;
    const TEST_INODE_BITMAP: u64 = 9;
    fn poke_bit(mut disk: SharedDisk, block: u64, bit: u64) {
        let mut data = disk.peek(block);
        data[(bit / 8) as usize] |= 1 << (bit % 8);
        disk.poke(block, &data);
    }

    fn clear_bit(mut disk: SharedDisk, block: u64, bit: u64) {
        let mut data = disk.peek(block);
        data[(bit / 8) as usize] &= !(1 << (bit % 8));
        disk.poke(block, &data);
    }

    fn bit_set(disk: &SharedDisk, block: u64, bit: u64) -> bool {
        let data = disk.peek(block);
        data[(bit / 8) as usize] & (1 << (bit % 8)) != 0
    }

    #[test]
    fn check_reports_clean_on_fresh_volume() {
        let mut fs = format_shared(SharedDisk::new(SMALL_BLOCKS));
        let report = fs.check().unwrap();
        assert_eq!(report, Report::default());
        let writes = fs.dev.write_count();
        let report = fs.check().unwrap();
        assert_eq!(report, Report::default());
        assert_eq!(fs.dev.write_count(), writes);
    }

    #[test]
    fn check_clears_leaked_block_bit() {
        let disk = SharedDisk::new(SMALL_BLOCKS);
        let mut fs = format_shared(disk.clone());
        let h = fs.create(b"/keep").unwrap();
        fs.write(h, 0, &vec![7u8; BLOCK_BYTES]).unwrap();
        fs.close(h).unwrap();
        fs.unmount().unwrap();
        poke_bit(disk.clone(), TEST_BLOCK_BITMAP, EXPECTED_DATA_START + 50);
        let mut fs = FileSystem::mount(disk.clone()).unwrap();
        let report = fs.check().unwrap();
        assert_eq!(report.leaked_blocks, 1);
        assert!(report.fixed);
        assert!(!report.hard_corrupt);
        assert_eq!(fs.free_blocks(), 972);
        fs.unmount().unwrap();
        assert!(!bit_set(&disk, TEST_BLOCK_BITMAP, EXPECTED_DATA_START + 50));
    }

    #[test]
    fn check_clears_stale_referenced_bit_direction_too() {
        let disk = SharedDisk::new(SMALL_BLOCKS);
        let mut fs = format_shared(disk.clone());
        let h = fs.create(b"/keep").unwrap();
        fs.write(h, 0, &vec![7u8; BLOCK_BYTES]).unwrap();
        fs.close(h).unwrap();
        fs.unmount().unwrap();
        clear_bit(disk.clone(), TEST_BLOCK_BITMAP, EXPECTED_DATA_START);
        let mut fs = FileSystem::mount(disk.clone()).unwrap();
        let report = fs.check().unwrap();
        assert_eq!(report.leaked_blocks, 0);
        assert!(report.fixed);
        assert_eq!(fs.free_blocks(), 972);
        fs.unmount().unwrap();
        assert!(bit_set(&disk, TEST_BLOCK_BITMAP, EXPECTED_DATA_START));
        let mut fs = FileSystem::mount(disk.clone()).unwrap();
        assert_eq!(fs.lookup(b"/keep").unwrap().size, BLOCK_BYTES as u64);
    }

    #[test]
    fn check_frees_leaked_inode() {
        let disk = SharedDisk::new(SMALL_BLOCKS);
        let fs = format_shared(disk.clone());
        let (table_block, slot_off) = inode_slot_location(&fs.geometry, 30);
        fs.unmount().unwrap();
        poke_bit(disk.clone(), TEST_INODE_BITMAP, 30);
        let mut fs = FileSystem::mount(disk.clone()).unwrap();
        let report = fs.check().unwrap();
        assert_eq!(report.leaked_inodes, 1);
        assert!(report.fixed);
        assert_eq!(fs.free_inodes(), 238);
        fs.unmount().unwrap();
        assert!(!bit_set(&disk, TEST_INODE_BITMAP, 30));
        let table = disk.peek(table_block);
        let slot: &[u8; INODE_SIZE] = table[slot_off..slot_off + INODE_SIZE]
            .try_into()
            .unwrap();
        assert_eq!(inode::generation_of(slot), 1);
    }

    #[test]
    fn check_removes_dangling_entry() {
        let mut disk = SharedDisk::new(SMALL_BLOCKS);
        let mut fs = format_shared(disk.clone());
        fs.create(b"/ghost").unwrap();
        let ghost = fs.lookup(b"/ghost").unwrap().inode;
        let (table_block, slot_off) = inode_slot_location(&fs.geometry, ghost);
        fs.unmount().unwrap();
        let mut table = disk.peek(table_block);
        for byte in &mut table[slot_off..slot_off + INODE_SIZE] {
            *byte = 0;
        }
        disk.poke(table_block, &table);
        let mut fs = FileSystem::mount(disk.clone()).unwrap();
        let report = fs.check().unwrap();
        assert_eq!(report.dangling_entries, 1);
        assert_eq!(report.leaked_inodes, 1);
        assert!(report.fixed);
        assert_eq!(fs.readdir(b"/").unwrap().len(), 2);
        fs.unmount().unwrap();
        let mut fs = FileSystem::mount(disk).unwrap();
        assert_eq!(fs.lookup(b"/ghost").err(), Some(FsError::NotFound));
        assert_eq!(fs.free_inodes(), 238);
    }

    #[test]
    fn check_strips_double_referenced_extent() {
        let mut disk = SharedDisk::new(SMALL_BLOCKS);
        let mut fs = format_shared(disk.clone());
        let f = fs.create(b"/f").unwrap();
        let keep = pattern_bytes(BLOCK_BYTES, 11);
        fs.write(f, 0, &keep).unwrap();
        fs.close(f).unwrap();
        let g = fs.create(b"/g").unwrap();
        fs.write(g, 0, &vec![9u8; BLOCK_BYTES]).unwrap();
        fs.close(g).unwrap();
        let g_stat = fs.lookup(b"/g").unwrap();
        let (g_inode, mut g_extents) = fs.load_inode(g_stat.inode).unwrap();
        let f_stat = fs.lookup(b"/f").unwrap();
        let (_, f_extents) = fs.load_inode(f_stat.inode).unwrap();
        g_extents[0].start = f_extents[0].start;
        let (table_block, slot_off) = inode_slot_location(&fs.geometry, g_stat.inode);
        fs.unmount().unwrap();
        let mut table = disk.peek(table_block);
        let target: &mut [u8; INODE_SIZE] =
            (&mut table[slot_off..slot_off + INODE_SIZE]).try_into().unwrap();
        inode::encode(&g_inode, &g_extents, target);
        disk.poke(table_block, &table);
        let mut fs = FileSystem::mount(disk.clone()).unwrap();
        let report = fs.check().unwrap();
        assert_eq!(report.double_refs, 1);
        assert!(report.fixed);
        let st = fs.lookup(b"/f").unwrap();
        assert_eq!(st.size, BLOCK_BYTES as u64);
        let h = fs.open(b"/f").unwrap();
        let mut buf = vec![0u8; BLOCK_BYTES];
        fs.read(h, 0, &mut buf).unwrap();
        assert_eq!(buf, keep);
        assert_eq!(fs.lookup(b"/g").unwrap().size, 0);
        assert_eq!(report.leaked_blocks, 1);
    }

    #[test]
    fn check_flags_extent_into_metadata() {
        let mut disk = SharedDisk::new(SMALL_BLOCKS);
        let mut fs = format_shared(disk.clone());
        let h = fs.create(b"/bad").unwrap();
        fs.write(h, 0, &vec![3u8; BLOCK_BYTES]).unwrap();
        fs.close(h).unwrap();
        let stat = fs.lookup(b"/bad").unwrap();
        let (inode_value, mut extents) = fs.load_inode(stat.inode).unwrap();
        extents[0].start = 5;
        let (table_block, slot_off) = inode_slot_location(&fs.geometry, stat.inode);
        fs.unmount().unwrap();
        let mut table = disk.peek(table_block);
        let target: &mut [u8; INODE_SIZE] =
            (&mut table[slot_off..slot_off + INODE_SIZE]).try_into().unwrap();
        inode::encode(&inode_value, &extents, target);
        disk.poke(table_block, &table);
        let mut fs = FileSystem::mount(disk.clone()).unwrap();
        let report = fs.check().unwrap();
        assert!(report.hard_corrupt);
        assert_eq!(fs.lookup(b"/bad").unwrap().size, 0);
    }

    #[test]
    fn check_restores_wrong_dot_parent() {
        let mut disk = SharedDisk::new(SMALL_BLOCKS);
        let fs = format_shared(disk.clone());
        fs.unmount().unwrap();
        let mut root_block = disk.peek(EXPECTED_DATA_START);
        root_block[12..16].copy_from_slice(&7u32.to_le_bytes());
        disk.poke(EXPECTED_DATA_START, &root_block);
        let mut fs = FileSystem::mount(disk.clone()).unwrap();
        let report = fs.check().unwrap();
        assert!(report.fixed);
        assert!(!report.hard_corrupt);
        fs.unmount().unwrap();
        let root_block = disk.peek(EXPECTED_DATA_START);
        assert_eq!(u32::from_le_bytes(root_block[12..16].try_into().unwrap()), ROOT_INODE);
    }

    #[test]
    fn check_flags_unwell_formed_dir_block_without_panicking() {
        let mut disk = SharedDisk::new(SMALL_BLOCKS);
        let fs = format_shared(disk.clone());
        fs.unmount().unwrap();
        let zeroed = vec![0u8; BLOCK_BYTES];
        disk.poke(EXPECTED_DATA_START, &zeroed.try_into().unwrap());
        let mut fs = FileSystem::mount(disk).unwrap();
        let report = fs.check().unwrap();
        assert!(report.hard_corrupt);
    }

    fn shared_bytes(blocks: u64) -> Rc<RefCell<Vec<u8>>> {
        Rc::new(RefCell::new(vec![0u8; blocks as usize * BLOCK_BYTES]))
    }

    struct Stable {
        bytes: Rc<RefCell<Vec<u8>>>,
    }

    impl BlockDevice for Stable {
        fn block_count(&self) -> u64 {
            (self.bytes.borrow().len() / BLOCK_BYTES) as u64
        }

        fn read_block(&mut self, block: u64, buf: &mut [u8]) -> Result<(), crate::error::IoError> {
            let bytes = self.bytes.borrow();
            let start = block as usize * BLOCK_BYTES;
            if start + buf.len() > bytes.len() {
                return Err(crate::error::IoError::Device);
            }
            buf.copy_from_slice(&bytes[start..start + buf.len()]);
            Ok(())
        }

        fn write_block(&mut self, block: u64, data: &[u8]) -> Result<(), crate::error::IoError> {
            let mut bytes = self.bytes.borrow_mut();
            let start = block as usize * BLOCK_BYTES;
            if start + data.len() > bytes.len() {
                return Err(crate::error::IoError::Device);
            }
            bytes[start..start + data.len()].copy_from_slice(data);
            Ok(())
        }

        fn flush(&mut self) -> Result<(), crate::error::IoError> {
            Ok(())
        }
    }

    struct Faulty {
        bytes: Rc<RefCell<Vec<u8>>>,
        budget: Rc<Cell<usize>>,
        writes: Rc<Cell<usize>>,
    }

    impl BlockDevice for Faulty {
        fn block_count(&self) -> u64 {
            (self.bytes.borrow().len() / BLOCK_BYTES) as u64
        }

        fn read_block(&mut self, block: u64, buf: &mut [u8]) -> Result<(), crate::error::IoError> {
            Stable { bytes: self.bytes.clone() }.read_block(block, buf)
        }

        fn write_block(&mut self, block: u64, data: &[u8]) -> Result<(), crate::error::IoError> {
            if self.budget.get() == 0 {
                return Err(crate::error::IoError::Device);
            }
            self.budget.set(self.budget.get() - 1);
            self.writes.set(self.writes.get() + 1);
            Stable { bytes: self.bytes.clone() }.write_block(block, data)
        }

        fn flush(&mut self) -> Result<(), crate::error::IoError> {
            Ok(())
        }
    }

    struct Volatile {
        bytes: Rc<RefCell<Vec<u8>>>,
        pending: BTreeMap<u64, Vec<u8>>,
    }

    impl BlockDevice for Volatile {
        fn block_count(&self) -> u64 {
            (self.bytes.borrow().len() / BLOCK_BYTES) as u64
        }

        fn read_block(&mut self, block: u64, buf: &mut [u8]) -> Result<(), crate::error::IoError> {
            if let Some(cached) = self.pending.get(&block) {
                buf.copy_from_slice(cached);
                return Ok(());
            }
            Stable { bytes: self.bytes.clone() }.read_block(block, buf)
        }

        fn write_block(&mut self, block: u64, data: &[u8]) -> Result<(), crate::error::IoError> {
            self.pending.insert(block, data.to_vec());
            Ok(())
        }

        fn flush(&mut self) -> Result<(), crate::error::IoError> {
            let mut target = Stable { bytes: self.bytes.clone() };
            for (block, data) in core::mem::take(&mut self.pending) {
                target.write_block(block, &data)?;
            }
            Ok(())
        }
    }

    fn script<D: BlockDevice>(fs: &mut FileSystem<D>) -> bool {
        let mut ok = true;
        ok &= fs.mkdir(b"/a").is_ok();
        if let Ok(h) = fs.create(b"/a/f") {
            ok &= fs.write(h, 0, &pattern_bytes(3 * BLOCK_BYTES, 1)).is_ok();
            ok &= fs.close(h).is_ok();
        } else {
            ok = false;
        }
        ok &= fs.sync().is_ok();
        if let Ok(h) = fs.create(b"/b") {
            ok &= fs.write(h, 0, &pattern_bytes(2 * BLOCK_BYTES, 2)).is_ok();
            ok &= fs.close(h).is_ok();
        } else {
            ok = false;
        }
        ok &= fs.sync().is_ok();
        if let Ok(h) = fs.open(b"/a/f") {
            ok &= fs.write(h, 3 * BLOCK_BYTES as u64, &pattern_bytes(2 * BLOCK_BYTES, 3)).is_ok();
            ok &= fs.close(h).is_ok();
        } else {
            ok = false;
        }
        ok &= fs.mkdir(b"/c").is_ok();
        ok &= fs.remove(b"/a/f").is_ok();
        if let Ok(h) = fs.create(b"/e") {
            ok &= fs.write(h, 0, &pattern_bytes(6 * BLOCK_BYTES, 5)).is_ok();
            ok &= fs.close(h).is_ok();
        } else {
            ok = false;
        }
        if let Ok(h) = fs.create(b"/g") {
            ok &= fs.write(h, 0, &pattern_bytes(4 * BLOCK_BYTES, 7)).is_ok();
            ok &= fs.close(h).is_ok();
        } else {
            ok = false;
        }
        ok &= fs.remove(b"/b").is_ok();
        ok &= fs.sync().is_ok();
        if let Ok(h) = fs.create(b"/h") {
            ok &= fs.write(h, 0, &pattern_bytes(BLOCK_BYTES, 11)).is_ok();
            ok &= fs.close(h).is_ok();
        } else {
            ok = false;
        }
        ok
    }

    const KNOWN_FILES: [&[u8]; 5] = [b"/a/f", b"/b", b"/e", b"/g", b"/h"];

    fn allowed_byte(path: &[u8], offset: usize, byte: u8) -> bool {
        if byte == 0 {
            return true;
        }
        let segments: &[(usize, usize, u8)] = match path {
            b"/a/f" => &[(0, 3 * BLOCK_BYTES, 1), (3 * BLOCK_BYTES, 2 * BLOCK_BYTES, 3)],
            b"/b" => &[(0, 2 * BLOCK_BYTES, 2)],
            b"/e" => &[(0, 6 * BLOCK_BYTES, 5)],
            b"/g" => &[(0, 4 * BLOCK_BYTES, 7)],
            b"/h" => &[(0, BLOCK_BYTES, 11)],
            _ => return false,
        };
        segments.iter().any(|(start, len, seed)| {
            offset >= *start
                && offset < start + *len
                && byte == ((offset - start) as u8).wrapping_mul(31).wrapping_add(*seed)
        })
    }

    fn verify_tree<D: BlockDevice>(
        fs: &mut FileSystem<D>,
        dir: Vec<u8>,
        violations: &mut Vec<(Vec<u8>, usize, u8)>,
        unexpected: &mut Vec<Vec<u8>>,
    ) {
        let items = fs.readdir(&dir).unwrap();
        for item in items {
            if item.name == b"." || item.name == b".." {
                continue;
            }
            let mut full = dir.clone();
            if full != b"/" {
                full.push(b'/');
            }
            full.extend_from_slice(&item.name);
            if item.kind == Kind::Directory {
                verify_tree(fs, full, violations, unexpected);
                continue;
            }
            if !KNOWN_FILES.iter().any(|known| *known == full.as_slice()) {
                unexpected.push(full);
                continue;
            }
            let st = fs.lookup(&full).unwrap();
            let h = fs.open(&full).unwrap();
            let mut buf = vec![0u8; st.size as usize];
            if st.size > 0 {
                fs.read(h, 0, &mut buf).unwrap();
            }
            fs.close(h).unwrap();
            for (offset, byte) in buf.iter().enumerate() {
                if !allowed_byte(&full, offset, *byte) {
                    violations.push((full.clone(), offset, *byte));
                }
            }
        }
    }

    fn recover_and_verify(bytes: Rc<RefCell<Vec<u8>>>) {
        let mut fs =
            FileSystem::mount(Stable { bytes: bytes.clone() }).expect("crash must leave volume mountable");
        let first = fs.check().unwrap();
        assert!(!first.hard_corrupt);
        let second = fs.check().unwrap();
        assert_eq!(second, Report::default(), "repair must converge: {:?}", second);
        let mut violations: Vec<(Vec<u8>, usize, u8)> = Vec::new();
        let mut unexpected: Vec<Vec<u8>> = Vec::new();
        verify_tree(&mut fs, b"/".to_vec(), &mut violations, &mut unexpected);
        assert!(violations.is_empty(), "foreign bytes found: {:?}", &violations[..violations.len().min(4)]);
        assert!(unexpected.is_empty(), "unexpected files: {:?}", unexpected);
    }

    #[test]
    fn faulty_device_rejects_writes_after_budget() {
        let bytes = shared_bytes(4);
        let budget = Rc::new(Cell::new(1usize));
        let writes = Rc::new(Cell::new(0usize));
        let mut dev = Faulty { bytes, budget: budget.clone(), writes: writes.clone() };
        let block = vec![0xA5u8; BLOCK_BYTES];
        assert!(dev.write_block(1, &block).is_ok());
        assert_eq!(budget.get(), 0);
        assert!(dev.write_block(2, &block).is_err());
        assert_eq!(writes.get(), 1);
    }

    #[test]
    fn volatile_device_buffers_until_flush() {
        let bytes = shared_bytes(4);
        {
            let mut dev = Volatile { bytes: bytes.clone(), pending: BTreeMap::new() };
            let block = vec![0x5Au8; BLOCK_BYTES];
            dev.write_block(1, &block).unwrap();
            let mut probe = [0u8; BLOCK_BYTES];
            assert!(bytes.borrow()[BLOCK_BYTES..2 * BLOCK_BYTES].iter().all(|b| *b == 0));
            dev.read_block(1, &mut probe).unwrap();
            assert_eq!(probe, [0x5Au8; BLOCK_BYTES]);
            dev.flush().unwrap();
            assert!(bytes.borrow()[BLOCK_BYTES..2 * BLOCK_BYTES].iter().all(|b| *b == 0x5A));
        }
        {
            let mut dev = Volatile { bytes: bytes.clone(), pending: BTreeMap::new() };
            dev.write_block(2, &vec![0x33u8; BLOCK_BYTES]).unwrap();
        }
        assert!(bytes.borrow()[2 * BLOCK_BYTES..3 * BLOCK_BYTES].iter().all(|b| *b == 0));
    }

    #[test]
    fn crash_injection_every_write_prefix_stays_consistent() {
        let bytes = shared_bytes(SMALL_BLOCKS);
        FileSystem::format(Stable { bytes: bytes.clone() }, &default_params(b"crashvol")).unwrap();
        let budget = Rc::new(Cell::new(usize::MAX));
        let writes = Rc::new(Cell::new(0usize));
        let mut fs = FileSystem::mount(Faulty {
            bytes: bytes.clone(),
            budget: budget.clone(),
            writes: writes.clone(),
        })
        .unwrap();
        assert!(script(&mut fs));
        let total = writes.get();
        assert!(total > 10);
        for prefix in 0..=total + 1 {
            let bytes = shared_bytes(SMALL_BLOCKS);
            FileSystem::format(Stable { bytes: bytes.clone() }, &default_params(b"crashvol")).unwrap();
            let mut fs = FileSystem::mount(Faulty {
                bytes: bytes.clone(),
                budget: Rc::new(Cell::new(prefix)),
                writes: Rc::new(Cell::new(0)),
            });
            if let Ok(fs) = &mut fs {
                let _ = script(fs);
            }
            core::mem::forget(fs);
            recover_and_verify(bytes);
        }
    }

    #[test]
    fn volatile_crash_loses_only_unflushed_writes() {
        let bytes = shared_bytes(SMALL_BLOCKS);
        FileSystem::format(Stable { bytes: bytes.clone() }, &default_params(b"crashvol")).unwrap();
        {
            let mut fs = FileSystem::mount(Volatile { bytes: bytes.clone(), pending: BTreeMap::new() })
                .unwrap();
            assert!(script(&mut fs));
        }
        recover_and_verify(bytes);
    }

    struct Rng {
        state: u64,
    }

    impl Rng {
        fn new(seed: u64) -> Rng {
            Rng { state: seed }
        }

        fn next_u64(&mut self) -> u64 {
            self.state = self.state.wrapping_add(0x9E3779B97F4A7C15);
            let mut z = self.state;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
            z ^ (z >> 31)
        }

        fn below(&mut self, n: usize) -> usize {
            (self.next_u64() % n as u64) as usize
        }

        fn fill(&mut self, len: usize) -> Vec<u8> {
            (0..len).map(|_| (self.next_u64() & 0xFF) as u8 | 1).collect()
        }
    }

    struct Model {
        files: BTreeMap<Vec<u8>, Vec<u8>>,
        dirs: BTreeSet<Vec<u8>>,
    }

    impl Model {
        fn new() -> Model {
            let mut dirs = BTreeSet::new();
            dirs.insert(b"/".to_vec());
            Model { files: BTreeMap::new(), dirs }
        }

        fn parent(path: &[u8]) -> Vec<u8> {
            let cut = path.iter().rposition(|b| *b == b'/').unwrap();
            if cut == 0 { b"/".to_vec() } else { path[..cut].to_vec() }
        }

        fn exists(&self, path: &[u8]) -> bool {
            self.files.contains_key(path) || self.dirs.contains(path)
        }

        fn has_children(&self, dir: &[u8]) -> bool {
            let mut prefix = dir.to_vec();
            prefix.push(b'/');
            self.files.keys().any(|k| k.starts_with(&prefix))
                || self.dirs.iter().any(|k| k.starts_with(&prefix))
        }

        fn children_of(&self, dir: &[u8]) -> BTreeSet<(Vec<u8>, bool)> {
            let mut prefix = dir.to_vec();
            if !prefix.ends_with(b"/") {
                prefix.push(b'/');
            }
            let mut out = BTreeSet::new();
            for key in self.files.keys().chain(self.dirs.iter()) {
                if !key.starts_with(&prefix) || key == &prefix {
                    continue;
                }
                let rest = &key[prefix.len()..];
                if rest.contains(&b'/') {
                    continue;
                }
                let is_dir = self.dirs.contains(key);
                out.insert((rest.to_vec(), is_dir));
            }
            out
        }
    }

    const DIR_CANDIDATES: [&[u8]; 6] = [b"/a", b"/b", b"/c", b"/a/x", b"/b/y", b"/a/x/d"];
    const FILE_CANDIDATES: [&[u8]; 8] = [
        b"/f1", b"/f2", b"/a/fa", b"/a/fb", b"/a/x/fx", b"/b/fb", b"/b/y/fy", b"/c/z",
    ];
    const MAX_FILE_BYTES: u64 = 64 * 1024;

    fn property_round<D: BlockDevice>(mut fs: FileSystem<D>, seed: u64) {
        {
            let mut rng = Rng::new(seed.wrapping_mul(7919).wrapping_add(13));
            let mut model = Model::new();
            for step in 0..200usize {
                let op = rng.below(10);
                match op {
                    0 => {
                        let path = DIR_CANDIDATES[rng.below(DIR_CANDIDATES.len())];
                        let parent_ok = model.dirs.contains(&Model::parent(path));
                        let clash = model.exists(path);
                        let expected_ok = parent_ok && !clash;
                        let got = fs.mkdir(path);
                        assert_eq!(got.is_ok(), expected_ok, "seed {seed} step {step} mkdir {path:?}");
                        if expected_ok {
                            model.dirs.insert(path.to_vec());
                        }
                    }
                    1 => {
                        let path = DIR_CANDIDATES[rng.below(DIR_CANDIDATES.len())];
                        let is_dir = model.dirs.contains(path);
                        let expected_ok = is_dir && !model.has_children(path);
                        let got = fs.rmdir(path);
                        assert_eq!(got.is_ok(), expected_ok, "seed {seed} step {step} rmdir {path:?}");
                        if expected_ok {
                            model.dirs.remove(path);
                        }
                    }
                    2 => {
                        let path = FILE_CANDIDATES[rng.below(FILE_CANDIDATES.len())];
                        let parent_ok = model.dirs.contains(&Model::parent(path));
                        let clash = model.exists(path);
                        let expected_ok = parent_ok && !clash;
                        let got = fs.create(path);
                        assert_eq!(got.is_ok(), expected_ok, "seed {seed} step {step} create {path:?}");
                        if let Ok(h) = got {
                            fs.close(h).unwrap();
                            model.files.insert(path.to_vec(), Vec::new());
                        }
                    }
                    3 | 4 => {
                        let path = FILE_CANDIDATES[rng.below(FILE_CANDIDATES.len())];
                        let Some(content) = model.files.get(path) else {
                            assert!(fs.open(path).is_err(), "seed {seed} step {step} open missing");
                            continue;
                        };
                        let size = content.len();
                        let offset = if op == 4 || size == 0 {
                            size
                        } else {
                            rng.below(size + 1)
                        };
                        if offset as u64 + 2048 > MAX_FILE_BYTES {
                            continue;
                        }
                        let len = 1 + rng.below(2048);
                        let data = rng.fill(len);
                        let h = fs.open(path).unwrap();
                        let written = fs.write(h, offset as u64, &data);
                        fs.close(h).unwrap();
                        assert_eq!(written.is_ok(), true, "seed {seed} step {step} write {path:?}");
                        let buf = model.files.get_mut(path).unwrap();
                        let end = offset + data.len();
                        if buf.len() < end {
                            buf.resize(end, 0);
                        }
                        buf[offset..end].copy_from_slice(&data);
                        assert_eq!(fs.lookup(path).unwrap().size, buf.len() as u64);
                    }
                    5 => {
                        let path = FILE_CANDIDATES[rng.below(FILE_CANDIDATES.len())];
                        let Some(content) = model.files.get(path).cloned() else {
                            continue;
                        };
                        if content.is_empty() {
                            continue;
                        }
                        let h = fs.open(path).unwrap();
                        let mut buf = vec![0u8; content.len()];
                        let n = fs.read(h, 0, &mut buf).unwrap();
                        fs.close(h).unwrap();
                        assert_eq!(n, content.len(), "seed {seed} step {step} read len");
                        assert_eq!(buf, content, "seed {seed} step {step} read bytes {path:?}");
                    }
                    6 => {
                        let path = FILE_CANDIDATES[rng.below(FILE_CANDIDATES.len())];
                        let expected_ok = model.files.contains_key(path);
                        let got = fs.remove(path);
                        assert_eq!(got.is_ok(), expected_ok, "seed {seed} step {step} remove {path:?}");
                        if expected_ok {
                            model.files.remove(path);
                        }
                    }
                    7 => {
                        let path = FILE_CANDIDATES[rng.below(FILE_CANDIDATES.len())];
                        let got = fs.lookup(path);
                        match model.files.get(path) {
                            Some(content) => {
                                assert!(got.is_ok(), "seed {seed} step {step} lookup {path:?}");
                                assert_eq!(got.unwrap().size, content.len() as u64);
                            }
                            None => assert!(got.is_err(), "seed {seed} step {step} lookup ghost {path:?}"),
                        }
                    }
                    8 => {
                        let dir = if rng.below(2) == 0 { b"/".to_vec() } else {
                            DIR_CANDIDATES[rng.below(DIR_CANDIDATES.len())].to_vec()
                        };
                        let expected = model.children_of(&dir);
                        let present = expected
                            .iter()
                            .filter(|(name, _)| !(name.as_slice() == b"." || name.as_slice() == b".."))
                            .cloned()
                            .collect::<BTreeSet<(Vec<u8>, bool)>>();
                        let got = fs.readdir(&dir);
                        if model.dirs.contains(&dir) {
                            let items = got.unwrap();
                            let mut actual = BTreeSet::new();
                            for item in &items {
                                if item.name == b"." || item.name == b".." {
                                    continue;
                                }
                                actual.insert((item.name.to_vec(), item.kind == Kind::Directory));
                            }
                            assert_eq!(actual, present, "seed {seed} step {step} readdir {dir:?}");
                        } else {
                            assert!(got.is_err(), "seed {seed} step {step} readdir non-dir {dir:?}");
                        }
                    }
                    _ => {
                        fs.sync().unwrap();
                    }
                }
            }
            fs.sync().unwrap();
            let mut actual_files: BTreeMap<Vec<u8>, Vec<u8>> = BTreeMap::new();
            let mut actual_dirs: BTreeSet<Vec<u8>> = BTreeSet::new();
            actual_dirs.insert(b"/".to_vec());
            harvest(&mut fs, b"/".to_vec(), &mut actual_files, &mut actual_dirs);
            assert_eq!(actual_dirs, model.dirs, "seed {seed} dir set");
            assert_eq!(
                actual_files.keys().collect::<Vec<_>>(),
                model.files.keys().collect::<Vec<_>>(),
                "seed {seed} file set"
            );
            for (path, content) in &model.files {
                assert_eq!(&actual_files[path], content, "seed {seed} content {path:?}");
            }
        }
    }

    #[test]
    fn differential_property_against_reference_model() {
        for seed in 1..=8u64 {
            let fs = FileSystem::format(
                Stable { bytes: shared_bytes(SMALL_BLOCKS) },
                &default_params(b"prop"),
            )
            .unwrap();
            property_round(fs, seed);
        }
    }

    #[cfg(feature = "std")]
    mod host_file {
        use super::*;
        use crate::file::FileDevice;

        fn tmp_image(name: &str) -> std::path::PathBuf {
            let mut path = std::path::PathBuf::from("/tmp/opencode");
            path.push(format!("rxfs-{name}.img"));
            path
        }

        fn layout_ops<D: BlockDevice>(fs: &mut FileSystem<D>) {
            fs.mkdir(b"/a").unwrap();
            let h = fs.create(b"/doc").unwrap();
            fs.write(h, 0, &pattern_bytes(70_000, 4)).unwrap();
            fs.close(h).unwrap();
            let h = fs.create(b"/a/inner").unwrap();
            fs.write(h, 0, b"inside").unwrap();
            fs.close(h).unwrap();
            fs.mkdir(b"/temp").unwrap();
            fs.remove(b"/a/inner").unwrap();
            fs.rmdir(b"/temp").unwrap();
            fs.sync().unwrap();
        }

        #[test]
        fn volume_matrix_on_host_file() {
            let path = tmp_image("matrix");
            let dev = FileDevice::create(&path, SMALL_BLOCKS).unwrap();
            let mut fs = FileSystem::format(dev, &default_params(b"hostvol")).unwrap();
            layout_ops(&mut fs);
            fs.unmount().unwrap();
            let mut fs = FileSystem::mount(FileDevice::open(&path).unwrap()).unwrap();
            assert_eq!(fs.lookup(b"/doc").unwrap().size, 70_000);
            let h = fs.open(b"/doc").unwrap();
            let mut buf = vec![0u8; 70_000];
            fs.read(h, 0, &mut buf).unwrap();
            assert_eq!(buf, pattern_bytes(70_000, 4));
            fs.close(h).unwrap();
            assert_eq!(fs.lookup(b"/a/inner").err(), Some(FsError::NotFound));
            assert_eq!(fs.readdir(b"/a").unwrap().len(), 2);
            let report = fs.check().unwrap();
            assert_eq!(report, Report::default());
            fs.unmount().unwrap();
            std::fs::remove_file(&path).unwrap();
        }

        #[test]
        fn host_file_image_matches_memory_image() {
            let path = tmp_image("match");
            let mut host =
                FileSystem::format(FileDevice::create(&path, SMALL_BLOCKS).unwrap(), &default_params(b"same"))
                    .unwrap();
            layout_ops(&mut host);
            host.unmount().unwrap();
            let bytes = shared_bytes(SMALL_BLOCKS);
            let mut mem = FileSystem::format(
                Stable { bytes: bytes.clone() },
                &default_params(b"same"),
            )
            .unwrap();
            layout_ops(&mut mem);
            mem.unmount().unwrap();
            let on_disk = std::fs::read(&path).unwrap();
            assert_eq!(on_disk, *bytes.borrow());
            std::fs::remove_file(&path).unwrap();
        }

        #[test]
        fn property_seeds_replay_on_host_file() {
            for seed in 1..=3u64 {
                let path = tmp_image(&format!("prop{seed}"));
                let fs = FileSystem::format(
                    FileDevice::create(&path, SMALL_BLOCKS).unwrap(),
                    &default_params(b"prop"),
                )
                .unwrap();
                property_round(fs, seed);
                std::fs::remove_file(&path).unwrap();
            }
        }
    }

    fn harvest<D: BlockDevice>(
        fs: &mut FileSystem<D>,
        dir: Vec<u8>,
        files: &mut BTreeMap<Vec<u8>, Vec<u8>>,
        dirs: &mut BTreeSet<Vec<u8>>,
    ) {
        let items = fs.readdir(&dir).unwrap();
        for item in items {
            if item.name == b"." || item.name == b".." {
                continue;
            }
            let mut full = dir.clone();
            if full != b"/" {
                full.push(b'/');
            }
            full.extend_from_slice(&item.name);
            if item.kind == Kind::Directory {
                dirs.insert(full.clone());
                harvest(fs, full, files, dirs);
            } else {
                let st = fs.lookup(&full).unwrap();
                let h = fs.open(&full).unwrap();
                let mut buf = vec![0u8; st.size as usize];
                if st.size > 0 {
                    fs.read(h, 0, &mut buf).unwrap();
                }
                fs.close(h).unwrap();
                files.insert(full, buf);
            }
        }
    }
}
