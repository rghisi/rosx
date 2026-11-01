use crate::checksum::fletcher32;
use crate::error::FsError;
use crate::spec::{
    Geometry, BLOCK_BYTES, BLOCKS_PER_BITMAP_BLOCK, FS_MAGIC, INODES_PER_TABLE_BLOCK, INODE_SIZE,
    RESERVED_PREFIX_BLOCKS, ROOT_INODE, VERSION,
};

pub const CHECK_CLEAN: u8 = 0;
pub const CHECK_UNCLEAN: u8 = 1;
pub const UUID_BYTES: usize = 16;
pub const VOLUME_NAME_BYTES: usize = 32;

pub const OFFSET_MAGIC: usize = 0;
pub const OFFSET_VERSION: usize = 8;
pub const OFFSET_BLOCK_SIZE: usize = 12;
pub const OFFSET_TOTAL_BLOCKS: usize = 16;
pub const OFFSET_INODE_COUNT: usize = 24;
pub const OFFSET_INODE_SIZE: usize = 28;
pub const OFFSET_FLAGS: usize = 30;
pub const OFFSET_BLOCK_BITMAP_START: usize = 32;
pub const OFFSET_BLOCK_BITMAP_BLOCKS: usize = 40;
pub const OFFSET_INODE_BITMAP_START: usize = 44;
pub const OFFSET_INODE_BITMAP_BLOCKS: usize = 52;
pub const OFFSET_INODE_TABLE_START: usize = 56;
pub const OFFSET_INODE_TABLE_BLOCKS: usize = 64;
pub const OFFSET_ROOT_INODE: usize = 72;
pub const OFFSET_MOUNT_STATE: usize = 76;
pub const OFFSET_UUID: usize = 80;
pub const OFFSET_VOLUME_NAME: usize = 96;
pub const OFFSET_CREATED_AT: usize = 128;
pub const OFFSET_TXG: usize = 136;
pub const OFFSET_FREE_BLOCKS: usize = 144;
pub const OFFSET_FREE_INODES: usize = 152;
pub const OFFSET_ALLOC_HINT: usize = 160;
pub const OFFSET_CHECKSUM: usize = 164;
pub const PAYLOAD_END: usize = 168;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Superblock {
    pub total_blocks: u64,
    pub inode_count: u32,
    pub block_bitmap_start: u64,
    pub block_bitmap_blocks: u32,
    pub inode_bitmap_start: u64,
    pub inode_bitmap_blocks: u32,
    pub inode_table_start: u64,
    pub inode_table_blocks: u32,
    pub mount_state: u8,
    pub uuid: [u8; UUID_BYTES],
    pub volume_name: [u8; VOLUME_NAME_BYTES],
    pub created_at: u64,
    pub txg: u64,
    pub free_blocks: u64,
    pub free_inodes: u32,
    pub alloc_hint: u32,
}

impl Superblock {
    pub fn from_geometry(geometry: &Geometry) -> Result<Superblock, FsError> {
        let fits = |value: u64| u32::try_from(value).is_ok();
        if !fits(geometry.inode_count)
            || !fits(geometry.block_bitmap_blocks)
            || !fits(geometry.inode_bitmap_blocks)
            || !fits(geometry.inode_table_blocks)
        {
            return Err(FsError::VolumeTooLarge);
        }
        Ok(Superblock {
            total_blocks: geometry.total_blocks,
            inode_count: geometry.inode_count as u32,
            block_bitmap_start: geometry.block_bitmap_start,
            block_bitmap_blocks: geometry.block_bitmap_blocks as u32,
            inode_bitmap_start: geometry.inode_bitmap_start,
            inode_bitmap_blocks: geometry.inode_bitmap_blocks as u32,
            inode_table_start: geometry.inode_table_start,
            inode_table_blocks: geometry.inode_table_blocks as u32,
            mount_state: CHECK_CLEAN,
            uuid: [0u8; UUID_BYTES],
            volume_name: [0u8; VOLUME_NAME_BYTES],
            created_at: 0,
            txg: 0,
            free_blocks: geometry.data_blocks,
            free_inodes: geometry.inode_count as u32 - 2,
            alloc_hint: 0,
        })
    }

    pub fn encode(&self, block: &mut [u8; BLOCK_BYTES]) {
        block.fill(0);
        block[OFFSET_MAGIC..OFFSET_MAGIC + 8].copy_from_slice(FS_MAGIC);
        write_u32(block, OFFSET_VERSION, VERSION);
        write_u32(block, OFFSET_BLOCK_SIZE, BLOCK_BYTES as u32);
        write_u64(block, OFFSET_TOTAL_BLOCKS, self.total_blocks);
        write_u32(block, OFFSET_INODE_COUNT, self.inode_count);
        write_u16(block, OFFSET_INODE_SIZE, INODE_SIZE as u16);
        write_u64(block, OFFSET_BLOCK_BITMAP_START, self.block_bitmap_start);
        write_u32(
            block,
            OFFSET_BLOCK_BITMAP_BLOCKS,
            self.block_bitmap_blocks,
        );
        write_u64(block, OFFSET_INODE_BITMAP_START, self.inode_bitmap_start);
        write_u32(
            block,
            OFFSET_INODE_BITMAP_BLOCKS,
            self.inode_bitmap_blocks,
        );
        write_u64(block, OFFSET_INODE_TABLE_START, self.inode_table_start);
        write_u32(block, OFFSET_INODE_TABLE_BLOCKS, self.inode_table_blocks);
        write_u32(block, OFFSET_ROOT_INODE, ROOT_INODE);
        block[OFFSET_MOUNT_STATE] = self.mount_state;
        block[OFFSET_UUID..OFFSET_UUID + UUID_BYTES].copy_from_slice(&self.uuid);
        block[OFFSET_VOLUME_NAME..OFFSET_VOLUME_NAME + VOLUME_NAME_BYTES]
            .copy_from_slice(&self.volume_name);
        write_u64(block, OFFSET_CREATED_AT, self.created_at);
        write_u64(block, OFFSET_TXG, self.txg);
        write_u64(block, OFFSET_FREE_BLOCKS, self.free_blocks);
        write_u32(block, OFFSET_FREE_INODES, self.free_inodes);
        write_u32(block, OFFSET_ALLOC_HINT, self.alloc_hint);
        let checksum = checksum_of(block);
        write_u32(block, OFFSET_CHECKSUM, checksum);
    }

    pub fn decode(block: &[u8; BLOCK_BYTES]) -> Result<Superblock, FsError> {
        if &block[OFFSET_MAGIC..OFFSET_MAGIC + 8] != FS_MAGIC {
            return Err(FsError::Corrupt);
        }
        if read_u32(block, OFFSET_VERSION) != VERSION {
            return Err(FsError::Corrupt);
        }
        if read_u32(block, OFFSET_BLOCK_SIZE) != BLOCK_BYTES as u32 {
            return Err(FsError::Corrupt);
        }
        if read_u16(block, OFFSET_INODE_SIZE) != INODE_SIZE as u16 {
            return Err(FsError::Corrupt);
        }
        if checksum_of(block) != read_u32(block, OFFSET_CHECKSUM) {
            return Err(FsError::Corrupt);
        }
        let mount_state = block[OFFSET_MOUNT_STATE];
        if mount_state != CHECK_CLEAN && mount_state != CHECK_UNCLEAN {
            return Err(FsError::Corrupt);
        }
        if read_u32(block, OFFSET_ROOT_INODE) != ROOT_INODE {
            return Err(FsError::Corrupt);
        }
        if block[OFFSET_FLAGS] != 0
            || block[OFFSET_FLAGS + 1] != 0
            || block[OFFSET_MOUNT_STATE + 1] != 0
            || block[OFFSET_UUID - 2..OFFSET_UUID] != [0, 0]
            || block[OFFSET_ALLOC_HINT - 4..OFFSET_ALLOC_HINT] != [0, 0, 0, 0]
            || block[PAYLOAD_END..] != [0; BLOCK_BYTES - PAYLOAD_END]
        {
            return Err(FsError::Corrupt);
        }
        let total_blocks = read_u64(block, OFFSET_TOTAL_BLOCKS);
        let inode_count = read_u32(block, OFFSET_INODE_COUNT);
        let block_bitmap_start = read_u64(block, OFFSET_BLOCK_BITMAP_START);
        let block_bitmap_blocks = read_u32(block, OFFSET_BLOCK_BITMAP_BLOCKS);
        let inode_bitmap_start = read_u64(block, OFFSET_INODE_BITMAP_START);
        let inode_bitmap_blocks = read_u32(block, OFFSET_INODE_BITMAP_BLOCKS);
        let inode_table_start = read_u64(block, OFFSET_INODE_TABLE_START);
        let inode_table_blocks = read_u32(block, OFFSET_INODE_TABLE_BLOCKS);
        if total_blocks == 0 || total_blocks > u32::MAX as u64 || !regions_are_well_formed(
            total_blocks,
            inode_count,
            block_bitmap_start,
            block_bitmap_blocks,
            inode_bitmap_start,
            inode_bitmap_blocks,
            inode_table_start,
            inode_table_blocks,
        ) {
            return Err(FsError::Corrupt);
        }
        let mut uuid = [0u8; UUID_BYTES];
        uuid.copy_from_slice(&block[OFFSET_UUID..OFFSET_UUID + UUID_BYTES]);
        let mut volume_name = [0u8; VOLUME_NAME_BYTES];
        volume_name
            .copy_from_slice(&block[OFFSET_VOLUME_NAME..OFFSET_VOLUME_NAME + VOLUME_NAME_BYTES]);
        Ok(Superblock {
            total_blocks,
            inode_count,
            block_bitmap_start,
            block_bitmap_blocks,
            inode_bitmap_start,
            inode_bitmap_blocks,
            inode_table_start,
            inode_table_blocks,
            mount_state,
            uuid,
            volume_name,
            created_at: read_u64(block, OFFSET_CREATED_AT),
            txg: read_u64(block, OFFSET_TXG),
            free_blocks: read_u64(block, OFFSET_FREE_BLOCKS),
            free_inodes: read_u32(block, OFFSET_FREE_INODES),
            alloc_hint: read_u32(block, OFFSET_ALLOC_HINT),
        })
    }
}

fn regions_are_well_formed(
    total_blocks: u64,
    inode_count: u32,
    block_bitmap_start: u64,
    block_bitmap_blocks: u32,
    inode_bitmap_start: u64,
    inode_bitmap_blocks: u32,
    inode_table_start: u64,
    inode_table_blocks: u32,
) -> bool {
    if block_bitmap_start != RESERVED_PREFIX_BLOCKS {
        return false;
    }
    if inode_count == 0
        || u64::from(inode_count) % INODES_PER_TABLE_BLOCK != 0
        || u64::from(inode_table_blocks) * BLOCK_BYTES as u64
            != u64::from(inode_count) * INODE_SIZE as u64
    {
        return false;
    }
    if u64::from(block_bitmap_blocks) != total_blocks.div_ceil(BLOCKS_PER_BITMAP_BLOCK)
        || u64::from(inode_bitmap_blocks)
            != u64::from(inode_count).div_ceil(BLOCKS_PER_BITMAP_BLOCK)
    {
        return false;
    }
    inode_bitmap_start == block_bitmap_start + u64::from(block_bitmap_blocks)
        && inode_table_start == inode_bitmap_start + u64::from(inode_bitmap_blocks)
        && inode_table_start + u64::from(inode_table_blocks) < total_blocks
}

fn checksum_of(block: &[u8; BLOCK_BYTES]) -> u32 {
    let mut scratch = *block;
    scratch[OFFSET_CHECKSUM..OFFSET_CHECKSUM + 4].fill(0);
    fletcher32(&scratch)
}

fn read_u16(block: &[u8], offset: usize) -> u16 {
    let mut bytes = [0u8; 2];
    bytes.copy_from_slice(&block[offset..offset + 2]);
    u16::from_le_bytes(bytes)
}

fn read_u32(block: &[u8], offset: usize) -> u32 {
    let mut bytes = [0u8; 4];
    bytes.copy_from_slice(&block[offset..offset + 4]);
    u32::from_le_bytes(bytes)
}

fn read_u64(block: &[u8], offset: usize) -> u64 {
    let mut bytes = [0u8; 8];
    bytes.copy_from_slice(&block[offset..offset + 8]);
    u64::from_le_bytes(bytes)
}

fn write_u16(block: &mut [u8], offset: usize, value: u16) {
    block[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}

fn write_u32(block: &mut [u8], offset: usize, value: u32) {
    block[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn write_u64(block: &mut [u8], offset: usize, value: u64) {
    block[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::{DEFAULT_BYTES_PER_INODE, Geometry};

    fn sample_geometry() -> Geometry {
        Geometry::plan(47, DEFAULT_BYTES_PER_INODE).unwrap()
    }

    fn padded_volume_name() -> [u8; VOLUME_NAME_BYTES] {
        let mut name = [0u8; VOLUME_NAME_BYTES];
        name[..5].copy_from_slice(b"HELLO");
        name
    }

    fn sample_superblock() -> Superblock {
        Superblock {
            total_blocks: 47,
            inode_count: 64,
            block_bitmap_start: 8,
            block_bitmap_blocks: 1,
            inode_bitmap_start: 9,
            inode_bitmap_blocks: 1,
            inode_table_start: 10,
            inode_table_blocks: 4,
            mount_state: CHECK_UNCLEAN,
            uuid: [
                3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18,
            ],
            volume_name: padded_volume_name(),
            created_at: 0x0123_4567_89AB_CDEF,
            txg: 41,
            free_blocks: 30,
            free_inodes: 60,
            alloc_hint: 777,
        }
    }

    fn encoded_sample() -> [u8; BLOCK_BYTES] {
        let mut block = [0u8; BLOCK_BYTES];
        sample_superblock().encode(&mut block);
        block
    }

    fn refix_checksum(block: &mut [u8; BLOCK_BYTES]) {
        let checksum = checksum_of(block);
        write_u32(block, OFFSET_CHECKSUM, checksum);
    }

    #[test]
    fn header_constants_land_at_pinned_offsets() {
        let block = encoded_sample();
        assert_eq!(&block[0..8], FS_MAGIC);
        assert_eq!(&block[8..12], &1u32.to_le_bytes());
        assert_eq!(&block[12..16], &(BLOCK_BYTES as u32).to_le_bytes());
        assert_eq!(&block[28..30], &(INODE_SIZE as u16).to_le_bytes());
        assert_eq!(&block[30..32], &[0, 0]);
        assert_eq!(&block[72..76], &ROOT_INODE.to_le_bytes());
        assert_eq!(block[76], CHECK_UNCLEAN);
        assert_eq!(block[77], 0);
        assert_eq!(&block[78..80], &[0, 0]);
    }

    #[test]
    fn fields_land_at_pinned_offsets() {
        let block = encoded_sample();
        assert_eq!(&block[16..24], &47u64.to_le_bytes());
        assert_eq!(&block[24..28], &64u32.to_le_bytes());
        assert_eq!(&block[32..40], &8u64.to_le_bytes());
        assert_eq!(&block[40..44], &1u32.to_le_bytes());
        assert_eq!(&block[44..52], &9u64.to_le_bytes());
        assert_eq!(&block[52..56], &1u32.to_le_bytes());
        assert_eq!(&block[56..64], &10u64.to_le_bytes());
        assert_eq!(&block[64..68], &4u32.to_le_bytes());
        assert_eq!(&block[80..96], &[3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18]);
        assert_eq!(&block[96..101], b"HELLO");
        assert!(block[101..128].iter().all(|&b| b == 0));
        assert_eq!(&block[128..136], &0x0123_4567_89AB_CDEFu64.to_le_bytes());
        assert_eq!(&block[136..144], &41u64.to_le_bytes());
        assert_eq!(&block[144..152], &30u64.to_le_bytes());
        assert_eq!(&block[152..156], &60u32.to_le_bytes());
        assert_eq!(&block[156..160], &[0, 0, 0, 0]);
        assert_eq!(&block[160..164], &777u32.to_le_bytes());
        assert!(block[168..].iter().all(|&b| b == 0));
    }

    #[test]
    fn checksum_field_is_nonzero_and_consistent() {
        let block = encoded_sample();
        assert_ne!(read_u32(&block, OFFSET_CHECKSUM), 0);
        assert_eq!(
            read_u32(&block, OFFSET_CHECKSUM),
            checksum_of(&block)
        );
    }

    #[test]
    fn decode_roundtrips_sample() {
        let block = encoded_sample();
        assert_eq!(Superblock::decode(&block), Ok(sample_superblock()));
    }

    #[test]
    fn from_geometry_produces_clean_defaults() {
        let sb = Superblock::from_geometry(&sample_geometry()).unwrap();
        let mut block = [0u8; BLOCK_BYTES];
        sb.encode(&mut block);
        assert_eq!(Superblock::decode(&block), Ok(sb));
        assert_eq!(sb.total_blocks, 47);
        assert_eq!(sb.inode_count, 64);
        assert_eq!(sb.mount_state, CHECK_CLEAN);
        assert_eq!(sb.txg, 0);
        assert_eq!(sb.free_blocks, 32);
        assert_eq!(sb.free_inodes, 62);
        assert_eq!(sb.alloc_hint, 0);
        assert_eq!(sb.uuid, [0u8; UUID_BYTES]);
        assert_eq!(sb.volume_name, [0u8; VOLUME_NAME_BYTES]);
    }

    #[test]
    fn from_geometry_rejects_oversized_handmade_geometry() {
        let geometry = Geometry {
            total_blocks: 100,
            inode_count: u64::from(u32::MAX) + 16,
            block_bitmap_start: RESERVED_PREFIX_BLOCKS,
            block_bitmap_blocks: 1,
            inode_bitmap_start: 9,
            inode_bitmap_blocks: 1,
            inode_table_start: 10,
            inode_table_blocks: 4,
            data_start: 14,
            data_blocks: 85,
            backup_superblock: 99,
        };
        assert_eq!(
            Superblock::from_geometry(&geometry),
            Err(FsError::VolumeTooLarge)
        );
    }

    #[test]
    fn flips_of_every_payload_byte_are_detected() {
        for offset in 0..PAYLOAD_END {
            let mut block = encoded_sample();
            block[offset] ^= 0x80;
            assert_eq!(
                Superblock::decode(&block),
                Err(FsError::Corrupt),
                "undetected flip at offset {offset}"
            );
        }
    }

    #[test]
    fn flips_in_the_tail_are_detected() {
        for offset in [PAYLOAD_END, 1000, 4095] {
            let mut block = encoded_sample();
            block[offset] ^= 0x01;
            assert_eq!(Superblock::decode(&block), Err(FsError::Corrupt));
        }
    }

    #[test]
    fn rejects_bad_identity_fields_even_with_valid_checksum() {
        for (offset, patch, length) in [
            (0usize, 0xFFu8, 1usize),
            (8, 0, 4),
            (12, 5, 4),
            (28, 7, 2),
        ] {
            let mut block = encoded_sample();
            block[offset..offset + length].fill(patch);
            refix_checksum(&mut block);
            assert_eq!(Superblock::decode(&block), Err(FsError::Corrupt));
        }
        let mut block = encoded_sample();
        block[..8].copy_from_slice(b"NOTMAGIC");
        refix_checksum(&mut block);
        assert_eq!(Superblock::decode(&block), Err(FsError::Corrupt));
    }

    #[test]
    fn rejects_unknown_mount_state() {
        let mut block = encoded_sample();
        block[OFFSET_MOUNT_STATE] = 2;
        refix_checksum(&mut block);
        assert_eq!(Superblock::decode(&block), Err(FsError::Corrupt));
    }

    #[test]
    fn accepts_clean_mount_state() {
        let mut sb = sample_superblock();
        sb.mount_state = CHECK_CLEAN;
        let mut block = [0u8; BLOCK_BYTES];
        sb.encode(&mut block);
        assert_eq!(Superblock::decode(&block), Ok(sb));
    }

    #[test]
    fn rejects_wrong_root_inode() {
        let mut block = encoded_sample();
        write_u32(&mut block, OFFSET_ROOT_INODE, 3);
        refix_checksum(&mut block);
        assert_eq!(Superblock::decode(&block), Err(FsError::Corrupt));
    }

    #[test]
    fn rejects_nonzero_reserved_and_flags() {
        for offset in [30usize, 31, 77, 78, 79, 156, 157, 158, 159] {
            let mut block = encoded_sample();
            block[offset] = 1;
            refix_checksum(&mut block);
            assert_eq!(
                Superblock::decode(&block),
                Err(FsError::Corrupt),
                "undetected nonzero at offset {offset}"
            );
        }
    }

    #[test]
    fn rejects_broken_region_layout_even_with_valid_checksum() {
        let violations: [(usize, usize, u64); 8] = [
            (OFFSET_BLOCK_BITMAP_START, 8, 7),
            (OFFSET_BLOCK_BITMAP_BLOCKS, 4, 0),
            (OFFSET_INODE_BITMAP_START, 8, 8),
            (OFFSET_INODE_BITMAP_BLOCKS, 4, 0),
            (OFFSET_INODE_TABLE_START, 8, 11),
            (OFFSET_INODE_TABLE_BLOCKS, 4, 3),
            (OFFSET_TOTAL_BLOCKS, 8, 13),
            (OFFSET_TOTAL_BLOCKS, 8, 1 << 32),
        ];
        for (offset, width, value) in violations {
            let mut block = encoded_sample();
            block[offset..offset + width].copy_from_slice(&value.to_le_bytes()[..width]);
            refix_checksum(&mut block);
            assert_eq!(
                Superblock::decode(&block),
                Err(FsError::Corrupt),
                "undetected layout violation at offset {offset}"
            );
        }
    }

    #[test]
    fn rejects_bad_inode_count_even_with_valid_checksum() {
        for count in [0u32, 63, 65, 80] {
            let mut block = encoded_sample();
            write_u32(&mut block, OFFSET_INODE_COUNT, count);
            refix_checksum(&mut block);
            assert_eq!(
                Superblock::decode(&block),
                Err(FsError::Corrupt),
                "undetected inode_count {count}"
            );
        }
    }

    #[test]
    fn decode_works_on_representative_geometries() {
        for total in [47u64, 1000, 262_144, u32::MAX as u64] {
            let geometry = Geometry::plan(total, DEFAULT_BYTES_PER_INODE).unwrap();
            let mut sb = Superblock::from_geometry(&geometry).unwrap();
            sb.txg = 5;
            sb.mount_state = CHECK_UNCLEAN;
            let mut block = [0u8; BLOCK_BYTES];
            sb.encode(&mut block);
            assert_eq!(Superblock::decode(&block), Ok(sb));
        }
    }
}
