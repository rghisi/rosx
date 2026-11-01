use crate::bitmap::BitMap;
use crate::cache::MetaCache;
use crate::device::BlockDevice;
use crate::dir;
use crate::error::FsError;
use crate::extent::Extent;
use crate::inode::{self, Inode, KIND_DIRECTORY, MODE_DIRECTORY};
use crate::spec::{
    BLOCK_BYTES, Geometry, INODE_SIZE, RESERVED_PREFIX_BLOCKS, ROOT_INODE, SUPERBLOCK_BLOCK,
};
use crate::superblock::{Superblock, CHECK_CLEAN, CHECK_UNCLEAN, UUID_BYTES, VOLUME_NAME_BYTES};

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
    unclean_previous: bool,
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
}
