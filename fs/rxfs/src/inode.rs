use crate::checksum::fletcher32;
use crate::error::FsError;
use crate::extent::{self, Extent};
use crate::spec::INODE_SIZE;
use alloc::vec::Vec;

pub const KIND_FREE: u8 = 0;
pub const KIND_REGULAR: u8 = 1;
pub const KIND_DIRECTORY: u8 = 2;

pub const OFFSET_CHECKSUM: usize = 0;
pub const OFFSET_KIND: usize = 4;
pub const OFFSET_FLAGS: usize = 5;
pub const OFFSET_MODE: usize = 6;
pub const OFFSET_LINKS: usize = 8;
pub const OFFSET_GENERATION: usize = 12;
pub const OFFSET_SIZE: usize = 16;
pub const OFFSET_MTIME: usize = 24;
pub const OFFSET_BLOCKS_USED: usize = 32;
pub const OFFSET_EXTENT_COUNT: usize = 36;
pub const OFFSET_SPILL_BLOCK: usize = 38;
pub const OFFSET_INLINE_EXTENTS: usize = 48;
pub const HEADER_BYTES: usize = 48;

pub const MODE_FILE: u16 = 0o644;
pub const MODE_DIRECTORY: u16 = 0o755;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Inode {
    pub kind: u8,
    pub mode: u16,
    pub links: u32,
    pub generation: u32,
    pub size: u64,
    pub mtime: u64,
    pub blocks_used: u32,
    pub extent_count: u16,
    pub spill_block: u32,
}

impl Inode {
    pub fn free(generation: u32) -> Inode {
        Inode {
            kind: KIND_FREE,
            mode: 0,
            links: 0,
            generation,
            size: 0,
            mtime: 0,
            blocks_used: 0,
            extent_count: 0,
            spill_block: 0,
        }
    }

    pub fn new(kind: u8, mode: u16, generation: u32) -> Inode {
        Inode {
            kind,
            mode,
            links: if kind == KIND_DIRECTORY { 2 } else { 1 },
            generation,
            size: 0,
            mtime: 0,
            blocks_used: 0,
            extent_count: 0,
            spill_block: 0,
        }
    }
}

pub fn encode(inode: &Inode, inline_extents: &[Extent], bytes: &mut [u8; INODE_SIZE]) {
    assert!(inline_extents.len() <= crate::spec::INLINE_EXTENTS);
    bytes.fill(0);
    bytes[OFFSET_KIND] = inode.kind;
    bytes[OFFSET_MODE..OFFSET_MODE + 2].copy_from_slice(&inode.mode.to_le_bytes());
    bytes[OFFSET_LINKS..OFFSET_LINKS + 4].copy_from_slice(&inode.links.to_le_bytes());
    bytes[OFFSET_GENERATION..OFFSET_GENERATION + 4]
        .copy_from_slice(&inode.generation.to_le_bytes());
    bytes[OFFSET_SIZE..OFFSET_SIZE + 8].copy_from_slice(&inode.size.to_le_bytes());
    bytes[OFFSET_MTIME..OFFSET_MTIME + 8].copy_from_slice(&inode.mtime.to_le_bytes());
    bytes[OFFSET_BLOCKS_USED..OFFSET_BLOCKS_USED + 4]
        .copy_from_slice(&inode.blocks_used.to_le_bytes());
    bytes[OFFSET_EXTENT_COUNT..OFFSET_EXTENT_COUNT + 2]
        .copy_from_slice(&inode.extent_count.to_le_bytes());
    bytes[OFFSET_SPILL_BLOCK..OFFSET_SPILL_BLOCK + 4]
        .copy_from_slice(&inode.spill_block.to_le_bytes());
    extent::encode_inline(inline_extents, &mut bytes[OFFSET_INLINE_EXTENTS..]);
    let checksum = checksum_of(bytes);
    bytes[OFFSET_CHECKSUM..OFFSET_CHECKSUM + 4].copy_from_slice(&checksum.to_le_bytes());
}

pub fn decode(bytes: &[u8; INODE_SIZE]) -> Result<(Inode, Vec<Extent>), FsError> {
    if checksum_of(bytes) != u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) {
        return Err(FsError::Corrupt);
    }
    let kind = bytes[OFFSET_KIND];
    if kind != KIND_FREE && kind != KIND_REGULAR && kind != KIND_DIRECTORY {
        return Err(FsError::Corrupt);
    }
    if bytes[OFFSET_FLAGS] != 0
        || bytes[HEADER_BYTES - 6..HEADER_BYTES] != [0, 0, 0, 0, 0, 0]
    {
        return Err(FsError::Corrupt);
    }
    let inline_extents = extent::decode_inline(&bytes[OFFSET_INLINE_EXTENTS..]);
    Ok((
        Inode {
            kind,
            mode: u16::from_le_bytes([bytes[OFFSET_MODE], bytes[OFFSET_MODE + 1]]),
            links: read_u32(bytes, OFFSET_LINKS),
            generation: read_u32(bytes, OFFSET_GENERATION),
            size: read_u64(bytes, OFFSET_SIZE),
            mtime: read_u64(bytes, OFFSET_MTIME),
            blocks_used: read_u32(bytes, OFFSET_BLOCKS_USED),
            extent_count: u16::from_le_bytes([bytes[OFFSET_EXTENT_COUNT], bytes[OFFSET_EXTENT_COUNT + 1]]),
            spill_block: read_u32(bytes, OFFSET_SPILL_BLOCK),
        },
        inline_extents,
    ))
}

pub fn generation_of(bytes: &[u8; INODE_SIZE]) -> u32 {
    read_u32(bytes, OFFSET_GENERATION)
}

pub fn is_free(bytes: &[u8; INODE_SIZE]) -> bool {
    bytes[OFFSET_KIND] == KIND_FREE
}

fn checksum_of(bytes: &[u8; INODE_SIZE]) -> u32 {
    let mut scratch = *bytes;
    scratch[OFFSET_CHECKSUM..OFFSET_CHECKSUM + 4].fill(0);
    fletcher32(&scratch)
}

fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ])
}

fn read_u64(bytes: &[u8], offset: usize) -> u64 {
    let mut value = [0u8; 8];
    value.copy_from_slice(&bytes[offset..offset + 8]);
    u64::from_le_bytes(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::INLINE_EXTENTS;
    use alloc::vec;

    fn sample_inode() -> Inode {
        Inode {
            kind: KIND_REGULAR,
            mode: 0o600,
            links: 3,
            generation: 11,
            size: 1_000_000,
            mtime: 77,
            blocks_used: 250,
            extent_count: 2,
            spill_block: 0,
        }
    }

    fn sample_extents() -> Vec<Extent> {
        vec![Extent { start: 500, length: 200 }, Extent { start: 900, length: 50 }]
    }

    #[test]
    fn fields_land_at_pinned_offsets() {
        let mut bytes = [0u8; INODE_SIZE];
        encode(&sample_inode(), &sample_extents(), &mut bytes);
        assert_eq!(bytes[OFFSET_KIND], KIND_REGULAR);
        assert_eq!(bytes[5], 0);
        assert_eq!(&bytes[6..8], &0o600u16.to_le_bytes());
        assert_eq!(&bytes[8..12], &3u32.to_le_bytes());
        assert_eq!(&bytes[12..16], &11u32.to_le_bytes());
        assert_eq!(&bytes[16..24], &1_000_000u64.to_le_bytes());
        assert_eq!(&bytes[24..32], &77u64.to_le_bytes());
        assert_eq!(&bytes[32..36], &250u32.to_le_bytes());
        assert_eq!(&bytes[36..38], &2u16.to_le_bytes());
        assert_eq!(&bytes[38..42], &0u32.to_le_bytes());
        assert!(bytes[42..48].iter().all(|&b| b == 0));
        assert_eq!(&bytes[48..56], &[0xF4, 0x01, 0x00, 0x00, 0xC8, 0x00, 0x00, 0x00]);
        assert_eq!(&bytes[56..64], &[0x84, 0x03, 0x00, 0x00, 0x32, 0x00, 0x00, 0x00]);
        assert!(bytes[64..INODE_SIZE].iter().all(|&b| b == 0));
    }

    #[test]
    fn decode_roundtrips_sample() {
        let mut bytes = [0u8; INODE_SIZE];
        encode(&sample_inode(), &sample_extents(), &mut bytes);
        let (inode, extents) = decode(&bytes).unwrap();
        assert_eq!(inode, sample_inode());
        assert_eq!(extents, sample_extents());
    }

    #[test]
    fn free_inode_roundtrips_with_generation() {
        let mut bytes = [0u8; INODE_SIZE];
        encode(&Inode::free(42), &[], &mut bytes);
        let (inode, extents) = decode(&bytes).unwrap();
        assert_eq!(inode, Inode::free(42));
        assert!(extents.is_empty());
        assert!(is_free(&bytes));
        assert_eq!(generation_of(&bytes), 42);
    }

    #[test]
    fn new_sets_link_and_mode_defaults() {
        let file = Inode::new(KIND_REGULAR, MODE_FILE, 5);
        assert_eq!(file.links, 1);
        assert_eq!(file.size, 0);
        let dir = Inode::new(KIND_DIRECTORY, MODE_DIRECTORY, 5);
        assert_eq!(dir.links, 2);
    }

    #[test]
    fn full_inline_capacity_roundtrips() {
        let extents: Vec<Extent> =
            (0..INLINE_EXTENTS).map(|i| Extent { start: (i * 10 + 1) as u32, length: 10 }).collect();
        let mut bytes = [0u8; INODE_SIZE];
        let mut inode = sample_inode();
        inode.extent_count = INLINE_EXTENTS as u16;
        encode(&inode, &extents, &mut bytes);
        let (decoded, decoded_extents) = decode(&bytes).unwrap();
        assert_eq!(decoded, inode);
        assert_eq!(decoded_extents, extents);
    }

    #[test]
    fn every_payload_byte_flip_breaks_the_checksum() {
        let mut bytes = [0u8; INODE_SIZE];
        encode(&sample_inode(), &sample_extents(), &mut bytes);
        for offset in 4..INODE_SIZE {
            let mut flipped = bytes;
            flipped[offset] ^= 0x01;
            assert!(
                decode(&flipped).is_err(),
                "undetected flip at offset {offset}"
            );
        }
    }

    #[test]
    fn rejects_unknown_kind_and_dirty_reserved_with_valid_checksum() {
        for offset in [4usize, 5, 42, 43, 44, 45, 46, 47] {
            let mut bytes = [0u8; INODE_SIZE];
            encode(&Inode::free(1), &[], &mut bytes);
            bytes[offset] = if offset == 4 { 9 } else { 1 };
            let checksum = checksum_of(&bytes);
            bytes[0..4].copy_from_slice(&checksum.to_le_bytes());
            assert!(decode(&bytes).is_err(), "offset {offset} accepted");
        }
    }

    #[test]
    fn generation_accessor_ignores_checksum() {
        let mut bytes = [0u8; INODE_SIZE];
        encode(&Inode::free(9), &[], &mut bytes);
        bytes[16] ^= 0xFF;
        assert!(decode(&bytes).is_err());
        assert_eq!(generation_of(&bytes), 9);
    }
}
