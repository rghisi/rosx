use crate::spec::{INLINE_EXTENTS, SPILL_EXTENTS};
use alloc::vec::Vec;

pub const EXTENT_BYTES: usize = 8;
pub const SPILL_HEADER_BYTES: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Extent {
    pub start: u32,
    pub length: u32,
}

impl Extent {
    pub fn end(&self) -> u64 {
        self.start as u64 + self.length as u64
    }
}

pub fn locate(extents: &[Extent], file_block: u64) -> Option<(Extent, u64)> {
    let mut base = 0u64;
    for extent in extents {
        let next = base + extent.length as u64;
        if file_block < next {
            return Some((*extent, file_block - base));
        }
        base = next;
    }
    None
}

pub fn total_blocks(extents: &[Extent]) -> u64 {
    extents
        .iter()
        .map(|extent| extent.length as u64)
        .sum()
}

pub fn append(extents: &mut Vec<Extent>, start: u32, length: u32) -> bool {
    if length == 0 {
        return true;
    }
    if let Some(last) = extents.last_mut() {
        if last.end() == start as u64 {
            let merged = last.length as u64 + length as u64;
            if merged > u32::MAX as u64 {
                return false;
            }
            last.length = merged as u32;
            return true;
        }
    }
    extents.push(Extent { start, length });
    true
}

pub fn is_ordered_non_adjacent(extents: &[Extent]) -> bool {
    let valid = |extent: &Extent| extent.start != 0 && extent.length > 0;
    extents.iter().all(valid)
        && extents.windows(2).all(|pair| pair[0].end() <= pair[1].start as u64)
}

pub fn split_inline_overflow(extents: &[Extent]) -> (&[Extent], &[Extent]) {
    extents.split_at(extents.len().min(INLINE_EXTENTS))
}

pub fn chunk_for_spill(extents: &[Extent]) -> Vec<&[Extent]> {
    extents.chunks(SPILL_EXTENTS).collect()
}

pub fn encode_inline(extents: &[Extent], slots: &mut [u8]) {
    assert!(extents.len() <= INLINE_EXTENTS && slots.len() >= INLINE_EXTENTS * EXTENT_BYTES);
    slots[..INLINE_EXTENTS * EXTENT_BYTES].fill(0);
    for (index, extent) in extents.iter().enumerate() {
        let base = index * EXTENT_BYTES;
        slots[base..base + 4].copy_from_slice(&extent.start.to_le_bytes());
        slots[base + 4..base + 8].copy_from_slice(&extent.length.to_le_bytes());
    }
}

pub fn decode_inline(slots: &[u8]) -> Vec<Extent> {
    decode_slots(&slots[..INLINE_EXTENTS * EXTENT_BYTES])
}

pub fn encode_spill_block(block: &mut [u8], next: u32, extents: &[Extent]) {
    assert!(block.len() >= SPILL_HEADER_BYTES + SPILL_EXTENTS * EXTENT_BYTES);
    assert!(extents.len() <= SPILL_EXTENTS);
    block[..SPILL_HEADER_BYTES + SPILL_EXTENTS * EXTENT_BYTES].fill(0);
    block[0..4].copy_from_slice(&next.to_le_bytes());
    block[4..6].copy_from_slice(&(extents.len() as u16).to_le_bytes());
    for (index, extent) in extents.iter().enumerate() {
        let base = SPILL_HEADER_BYTES + index * EXTENT_BYTES;
        block[base..base + 4].copy_from_slice(&extent.start.to_le_bytes());
        block[base + 4..base + 8].copy_from_slice(&extent.length.to_le_bytes());
    }
}

pub fn decode_spill_block(block: &[u8]) -> Option<(u32, Vec<Extent>)> {
    if block.len() < SPILL_HEADER_BYTES + SPILL_EXTENTS * EXTENT_BYTES {
        return None;
    }
    let count = u16::from_le_bytes([block[4], block[5]]) as usize;
    if count > SPILL_EXTENTS || block[6..8] != [0, 0] {
        return None;
    }
    let extents = decode_slots(&block[SPILL_HEADER_BYTES..SPILL_HEADER_BYTES + count * EXTENT_BYTES]);
    if extents.len() != count {
        return None;
    }
    let next = u32::from_le_bytes([block[0], block[1], block[2], block[3]]);
    Some((next, extents))
}

fn decode_slots(bytes: &[u8]) -> Vec<Extent> {
    let mut extents = Vec::new();
    for slot in bytes.chunks_exact(EXTENT_BYTES) {
        let start = u32::from_le_bytes([slot[0], slot[1], slot[2], slot[3]]);
        let length = u32::from_le_bytes([slot[4], slot[5], slot[6], slot[7]]);
        if start == 0 || length == 0 {
            break;
        }
        extents.push(Extent { start, length });
    }
    extents
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::BLOCK_BYTES;
    use alloc::vec;

    fn extent(start: u32, length: u32) -> Extent {
        Extent { start, length }
    }

    #[test]
    fn locate_finds_extent_and_inner_offset() {
        let extents = vec![extent(10, 5), extent(100, 3)];
        assert_eq!(locate(&extents, 0), Some((extent(10, 5), 0)));
        assert_eq!(locate(&extents, 4), Some((extent(10, 5), 4)));
        assert_eq!(locate(&extents, 5), Some((extent(100, 3), 0)));
        assert_eq!(locate(&extents, 7), Some((extent(100, 3), 2)));
        assert_eq!(locate(&extents, 8), None);
        assert_eq!(locate(&[], 0), None);
    }

    #[test]
    fn total_blocks_sums_extents() {
        assert_eq!(total_blocks(&[]), 0);
        assert_eq!(total_blocks(&[extent(10, 5), extent(100, 3)]), 8);
    }

    #[test]
    fn append_coalesces_adjacent_runs() {
        let mut extents = vec![extent(10, 5)];
        assert!(append(&mut extents, 15, 5));
        assert_eq!(extents, vec![extent(10, 10)]);
    }

    #[test]
    fn append_pushes_non_adjacent() {
        let mut extents = vec![extent(10, 5)];
        assert!(append(&mut extents, 20, 2));
        assert_eq!(extents, vec![extent(10, 5), extent(20, 2)]);
    }

    #[test]
    fn append_rejects_merge_overflow_and_ignores_zero_length() {
        let mut extents = vec![extent(0, u32::MAX)];
        assert!(!append(&mut extents, u32::MAX, 1));
        assert_eq!(extents, vec![extent(0, u32::MAX)]);
        let untouched = extents.clone();
        assert!(append(&mut extents, 999, 0));
        assert_eq!(extents, untouched);
    }

    #[test]
    fn append_to_empty_list_pushes() {
        let mut extents = Vec::new();
        assert!(append(&mut extents, 50, 7));
        assert_eq!(extents, vec![extent(50, 7)]);
    }

    #[test]
    fn ordering_predicate() {
        assert!(is_ordered_non_adjacent(&[]));
        assert!(is_ordered_non_adjacent(&[extent(5, 1), extent(7, 2)]));
        assert!(is_ordered_non_adjacent(&[extent(5, 1), extent(6, 2)]));
        assert!(!is_ordered_non_adjacent(&[extent(0, 1)]));
        assert!(!is_ordered_non_adjacent(&[extent(5, 0)]));
        assert!(!is_ordered_non_adjacent(&[extent(10, 2), extent(5, 2)]));
    }

    #[test]
    fn split_keeps_first_26_inline() {
        let extents: Vec<Extent> = (0..30).map(|i| extent(100 + i * 4, 4)).collect();
        let (inline, spill) = split_inline_overflow(&extents);
        assert_eq!(inline.len(), 26);
        assert_eq!(spill.len(), 4);
        let small = [extent(10, 1), extent(20, 1)];
        let (inline, spill) = split_inline_overflow(&small);
        assert_eq!(inline.len(), 2);
        assert!(spill.is_empty());
        let (inline, spill) = split_inline_overflow(&[]);
        assert!(inline.is_empty() && spill.is_empty());
    }

    #[test]
    fn chunking_splits_at_511() {
        let extents: Vec<Extent> = (0..1023).map(|i| extent(i + 1, 1)).collect();
        let chunks = chunk_for_spill(&extents);
        assert_eq!(chunks.len(), 3);
        assert_eq!(chunks[0].len(), 511);
        assert_eq!(chunks[2].len(), 1);
        assert!(chunk_for_spill(&[]).is_empty());
    }

    #[test]
    fn inline_roundtrip_zeroes_unused_slots() {
        let extents = vec![extent(10, 5), extent(100, 3)];
        let mut slots = vec![0xEEu8; INLINE_EXTENTS * EXTENT_BYTES];
        encode_inline(&extents, &mut slots);
        assert_eq!(slots[..16], [10, 0, 0, 0, 5, 0, 0, 0, 100, 0, 0, 0, 3, 0, 0, 0]);
        assert!(slots[16..].iter().all(|&b| b == 0));
        assert_eq!(decode_inline(&slots), extents);
    }

    #[test]
    fn decode_inline_stops_at_first_dead_slot() {
        let mut slots = vec![0u8; INLINE_EXTENTS * EXTENT_BYTES];
        encode_inline(&[extent(10, 5)], &mut slots);
        slots[8..12].copy_from_slice(&20u32.to_le_bytes());
        assert_eq!(decode_inline(&slots), vec![extent(10, 5)]);
    }

    #[test]
    fn spill_block_roundtrip() {
        let extents: Vec<Extent> = (0..511).map(|i| extent(i + 1, 2)).collect();
        let mut block = vec![0xEEu8; BLOCK_BYTES];
        encode_spill_block(&mut block, 42, &extents);
        let (next, decoded) = decode_spill_block(&block).unwrap();
        assert_eq!(next, 42);
        assert_eq!(decoded, extents);
        assert_eq!(
            &block[SPILL_HEADER_BYTES + 511 * EXTENT_BYTES..],
            &[0u8; BLOCK_BYTES - SPILL_HEADER_BYTES - SPILL_EXTENTS * EXTENT_BYTES][..]
        );
    }

    #[test]
    fn spill_block_rejects_corrupt_headers() {
        let mut block = vec![0u8; BLOCK_BYTES];
        encode_spill_block(&mut block, 0, &[]);
        assert_eq!(decode_spill_block(&block), Some((0, Vec::new())));

        let mut too_many = block.clone();
        too_many[4..6].copy_from_slice(&512u16.to_le_bytes());
        assert_eq!(decode_spill_block(&too_many), None);

        let mut dirty_reserved = block.clone();
        dirty_reserved[6] = 1;
        assert_eq!(decode_spill_block(&dirty_reserved), None);

        let mut lying_count = block.clone();
        lying_count[4..6].copy_from_slice(&1u16.to_le_bytes());
        assert_eq!(decode_spill_block(&lying_count), None);

        assert_eq!(decode_spill_block(&vec![0u8; BLOCK_BYTES - 1]), None);
    }

    #[test]
    fn slot_decode_rejects_dead_lengths_mid_list() {
        let mut block = vec![0u8; BLOCK_BYTES];
        encode_spill_block(&mut block, 7, &[extent(9, 1), extent(0, 1)]);
        block[4..6].copy_from_slice(&2u16.to_le_bytes());
        assert_eq!(decode_spill_block(&block), None);
    }
}
