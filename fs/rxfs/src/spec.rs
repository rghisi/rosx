use crate::error::FsError;

pub const BLOCK_BYTES: usize = 4096;
pub const SUPERBLOCK_BLOCK: u64 = 1;
pub const RESERVED_PREFIX_BLOCKS: u64 = 8;
pub const FS_MAGIC: &[u8; 8] = b"RXFSV1__";
pub const VERSION: u32 = 1;
pub const INODE_SIZE: usize = 256;
pub const INODES_PER_TABLE_BLOCK: u64 = 16;
pub const INLINE_EXTENTS: usize = 26;
pub const SPILL_EXTENTS: usize = 511;
pub const MAX_NAME_BYTES: usize = 255;
pub const MAX_PATH_BYTES: usize = 4096;
pub const MAX_DEPTH: usize = 512;
pub const MAX_OPEN: usize = 64;
pub const MIN_INODES: u64 = 64;
pub const MIN_DATA_BLOCKS: u64 = 32;
pub const DEFAULT_BYTES_PER_INODE: u32 = 16384;
pub const ROOT_INODE: u32 = 1;
pub const BLOCKS_PER_BITMAP_BLOCK: u64 = (BLOCK_BYTES as u64) * 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Geometry {
    pub total_blocks: u64,
    pub inode_count: u64,
    pub block_bitmap_start: u64,
    pub block_bitmap_blocks: u64,
    pub inode_bitmap_start: u64,
    pub inode_bitmap_blocks: u64,
    pub inode_table_start: u64,
    pub inode_table_blocks: u64,
    pub data_start: u64,
    pub data_blocks: u64,
    pub backup_superblock: u64,
}

impl Geometry {
    pub fn plan(total_blocks: u64, bytes_per_inode: u32) -> Result<Geometry, FsError> {
        if total_blocks == 0 {
            return Err(FsError::DeviceTooSmall);
        }
        if total_blocks > u32::MAX as u64 {
            return Err(FsError::VolumeTooLarge);
        }
        let bytes_per_inode = bytes_per_inode.max(BLOCK_BYTES as u32) as u64;
        let raw_inodes = total_blocks * BLOCK_BYTES as u64 / bytes_per_inode;
        let rounded_inodes =
            raw_inodes.div_euclid(INODES_PER_TABLE_BLOCK) * INODES_PER_TABLE_BLOCK;
        let inode_count = rounded_inodes.max(MIN_INODES);
        let block_bitmap_start = RESERVED_PREFIX_BLOCKS;
        let block_bitmap_blocks = total_blocks.div_ceil(BLOCKS_PER_BITMAP_BLOCK);
        let inode_bitmap_start = block_bitmap_start + block_bitmap_blocks;
        let inode_bitmap_blocks = inode_count.div_ceil(BLOCKS_PER_BITMAP_BLOCK);
        let inode_table_start = inode_bitmap_start + inode_bitmap_blocks;
        let inode_table_blocks = inode_count.div_euclid(INODES_PER_TABLE_BLOCK);
        let data_start = inode_table_start + inode_table_blocks;
        let backup_superblock = total_blocks - 1;
        let data_blocks = backup_superblock.saturating_sub(data_start);
        if data_blocks < MIN_DATA_BLOCKS {
            return Err(FsError::DeviceTooSmall);
        }
        Ok(Geometry {
            total_blocks,
            inode_count,
            block_bitmap_start,
            block_bitmap_blocks,
            inode_bitmap_start,
            inode_bitmap_blocks,
            inode_table_start,
            inode_table_blocks,
            data_start,
            data_blocks,
            backup_superblock,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fs_magic_is_pinned() {
        assert_eq!(FS_MAGIC, &[b'R', b'X', b'F', b'S', b'V', b'1', b'_', b'_']);
    }

    #[test]
    fn plan_one_gib_volume_with_default_density() {
        let geometry = Geometry::plan(262_144, DEFAULT_BYTES_PER_INODE).unwrap();
        assert_eq!(geometry.total_blocks, 262_144);
        assert_eq!(geometry.inode_count, 65_536);
        assert_eq!(geometry.block_bitmap_start, 8);
        assert_eq!(geometry.block_bitmap_blocks, 8);
        assert_eq!(geometry.inode_bitmap_start, 16);
        assert_eq!(geometry.inode_bitmap_blocks, 2);
        assert_eq!(geometry.inode_table_start, 18);
        assert_eq!(geometry.inode_table_blocks, 4096);
        assert_eq!(geometry.data_start, 4114);
        assert_eq!(geometry.backup_superblock, 262_143);
        assert_eq!(geometry.data_blocks, 258_029);
    }

    #[test]
    fn plan_rounds_inode_count_down_to_table_multiple() {
        let geometry = Geometry::plan(1000, 5000).unwrap();
        assert_eq!(geometry.inode_count, 816);
        assert_eq!(geometry.block_bitmap_start, 8);
        assert_eq!(geometry.block_bitmap_blocks, 1);
        assert_eq!(geometry.inode_bitmap_start, 9);
        assert_eq!(geometry.inode_bitmap_blocks, 1);
        assert_eq!(geometry.inode_table_start, 10);
        assert_eq!(geometry.inode_table_blocks, 51);
        assert_eq!(geometry.data_start, 61);
        assert_eq!(geometry.backup_superblock, 999);
        assert_eq!(geometry.data_blocks, 938);
    }

    #[test]
    fn plan_floors_inode_count_at_minimum() {
        let geometry = Geometry::plan(100, u32::MAX).unwrap();
        assert_eq!(geometry.inode_count, MIN_INODES);
        assert_eq!(geometry.inode_table_blocks, 4);
        assert!(geometry.data_blocks >= MIN_DATA_BLOCKS);
    }

    #[test]
    fn plan_clamps_bytes_per_inode_to_block_size() {
        let clamped = Geometry::plan(1000, 0).unwrap();
        assert_eq!(clamped, Geometry::plan(1000, BLOCK_BYTES as u32).unwrap());
        assert_eq!(
            Geometry::plan(1000, 100).unwrap(),
            Geometry::plan(1000, BLOCK_BYTES as u32).unwrap()
        );
        assert_eq!(clamped.inode_count, 992);
    }

    #[test]
    fn plan_rejects_empty_device() {
        assert_eq!(
            Geometry::plan(0, DEFAULT_BYTES_PER_INODE),
            Err(FsError::DeviceTooSmall)
        );
    }

    #[test]
    fn plan_rejects_device_without_min_data_blocks() {
        assert_eq!(
            Geometry::plan(46, DEFAULT_BYTES_PER_INODE),
            Err(FsError::DeviceTooSmall)
        );
        assert_eq!(
            Geometry::plan(47, DEFAULT_BYTES_PER_INODE),
            Ok(Geometry {
                total_blocks: 47,
                inode_count: 64,
                block_bitmap_start: 8,
                block_bitmap_blocks: 1,
                inode_bitmap_start: 9,
                inode_bitmap_blocks: 1,
                inode_table_start: 10,
                inode_table_blocks: 4,
                data_start: 14,
                data_blocks: 32,
                backup_superblock: 46,
            })
        );
    }

    #[test]
    fn plan_rejects_volume_beyond_u32_block_addressing() {
        assert_eq!(
            Geometry::plan(u32::MAX as u64 + 1, DEFAULT_BYTES_PER_INODE),
            Err(FsError::VolumeTooLarge)
        );
        assert!(Geometry::plan(u32::MAX as u64, DEFAULT_BYTES_PER_INODE).is_ok());
    }

    #[test]
    fn plan_accepts_maximum_addressable_volume() {
        let geometry = Geometry::plan(u32::MAX as u64, DEFAULT_BYTES_PER_INODE).unwrap();
        assert_eq!(geometry.block_bitmap_blocks, 131_072);
        assert_eq!(geometry.inode_count, 1_073_741_808);
        assert_eq!(geometry.inode_table_blocks, 67_108_863);
        assert_eq!(geometry.backup_superblock, u32::MAX as u64 - 1);
        assert_eq!(
            geometry.data_blocks,
            geometry.backup_superblock - geometry.data_start
        );
    }

    #[test]
    fn plan_regions_are_contiguous_and_ordered() {
        for (total, density) in [(47u64, 16384u32), (1000, 5000), (262_144, 16384)] {
            let geometry = Geometry::plan(total, density).unwrap();
            assert_eq!(geometry.block_bitmap_start, RESERVED_PREFIX_BLOCKS);
            assert_eq!(
                geometry.inode_bitmap_start,
                geometry.block_bitmap_start + geometry.block_bitmap_blocks
            );
            assert_eq!(
                geometry.inode_table_start,
                geometry.inode_bitmap_start + geometry.inode_bitmap_blocks
            );
            assert_eq!(
                geometry.data_start,
                geometry.inode_table_start + geometry.inode_table_blocks
            );
            assert_eq!(geometry.backup_superblock, total - 1);
            assert_eq!(
                geometry.data_blocks + geometry.data_start,
                geometry.backup_superblock
            );
            assert!(geometry.inode_count % INODES_PER_TABLE_BLOCK == 0);
            assert_eq!(
                geometry.inode_table_blocks * BLOCK_BYTES as u64,
                geometry.inode_count * INODE_SIZE as u64
            );
            assert!(
                geometry.block_bitmap_blocks * BLOCK_BYTES as u64 * 8 >= geometry.total_blocks
            );
            assert!(
                geometry.inode_bitmap_blocks * BLOCK_BYTES as u64 * 8 >= geometry.inode_count
            );
        }
    }
}
