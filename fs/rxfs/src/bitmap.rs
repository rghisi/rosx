use crate::device::BlockDevice;
use crate::error::FsError;
use crate::spec::BLOCK_BYTES;
use alloc::vec;
use alloc::vec::Vec;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BitMap {
    bits: Vec<u8>,
    bit_count: u64,
}

impl BitMap {
    pub fn empty(bit_count: u64) -> BitMap {
        let mut map = BitMap {
            bits: vec![0u8; byte_len(bit_count)],
            bit_count,
        };
        map.set_tail_bits_used();
        map
    }

    pub fn all_used(bit_count: u64) -> BitMap {
        let mut map = BitMap {
            bits: vec![0xFFu8; byte_len(bit_count)],
            bit_count,
        };
        map.set_tail_bits_used();
        map
    }

    pub fn is_used(&self, index: u64) -> bool {
        if index >= self.bit_count {
            return true;
        }
        self.bits[(index / 8) as usize] & (1 << (index % 8)) != 0
    }

    pub fn set_used(&mut self, index: u64) {
        if index < self.bit_count {
            self.bits[(index / 8) as usize] |= 1 << (index % 8);
        }
    }

    pub fn set_free(&mut self, index: u64) {
        if index < self.bit_count {
            self.bits[(index / 8) as usize] &= !(1 << (index % 8));
        }
    }

    pub fn use_range(&mut self, start: u64, len: u64) {
        for index in start..start.saturating_add(len) {
            self.set_used(index);
        }
    }

    pub fn free_range(&mut self, start: u64, len: u64) {
        for index in start..start.saturating_add(len) {
            self.set_free(index);
        }
    }

    pub fn free_count(&self) -> u64 {
        self.bits
            .iter()
            .map(|byte| (8 - byte.count_ones()) as u64)
            .sum()
    }

    pub fn first_free(&self, hint: u64) -> Option<u64> {
        if self.bit_count == 0 {
            return None;
        }
        let start = hint.min(self.bit_count - 1);
        if let Some(index) = self.first_free_in(start, self.bit_count) {
            return Some(index);
        }
        self.first_free_in(0, start)
    }

    pub fn allocate_one(&mut self, hint: u64) -> Option<u64> {
        let index = self.first_free(hint)?;
        self.set_used(index);
        Some(index)
    }

    pub fn allocate_run(&mut self, hint: u64, want: u64) -> Option<(u64, u64)> {
        let start = self.first_free(hint)?;
        let limit = start.saturating_add(want).min(self.bit_count);
        let mut end = start;
        while end < limit && !self.is_used(end) {
            end += 1;
        }
        self.use_range(start, end - start);
        Some((start, end - start))
    }

    pub fn load<D: BlockDevice>(
        dev: &mut D,
        region_start: u64,
        region_blocks: u64,
        bit_count: u64,
    ) -> Result<BitMap, FsError> {
        let mut bits = vec![0u8; byte_len(bit_count)];
        let mut block = [0u8; BLOCK_BYTES];
        let mut offset = 0usize;
        for region_index in 0..region_blocks {
            dev.read_block(region_start + region_index, &mut block)
                .map_err(FsError::Io)?;
            let take = BLOCK_BYTES.min(bits.len() - offset);
            bits[offset..offset + take].copy_from_slice(&block[..take]);
            offset += take;
        }
        let mut map = BitMap { bits, bit_count };
        map.set_tail_bits_used();
        Ok(map)
    }

    pub fn store<D: BlockDevice>(
        &self,
        dev: &mut D,
        region_start: u64,
    ) -> Result<(), FsError> {
        let mut block = [0u8; BLOCK_BYTES];
        let mut offset = 0usize;
        let mut region_index = 0u64;
        while offset < self.bits.len() {
            block.fill(0);
            let give = BLOCK_BYTES.min(self.bits.len() - offset);
            block[..give].copy_from_slice(&self.bits[offset..offset + give]);
            dev.write_block(region_start + region_index, &block)
                .map_err(FsError::Io)?;
            offset += give;
            region_index += 1;
        }
        Ok(())
    }

    fn first_free_in(&self, from: u64, to: u64) -> Option<u64> {
        (from..to).find(|&index| !self.is_used(index))
    }

    fn set_tail_bits_used(&mut self) {
        let whole_bytes = (self.bit_count / 8) as usize;
        let tail_bits = (self.bit_count % 8) as u32;
        if tail_bits != 0 {
            self.bits[whole_bytes] |= 0xFF << tail_bits;
        }
    }
}

fn byte_len(bit_count: u64) -> usize {
    ((bit_count + 7) / 8) as usize
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::MemoryDevice;

    #[test]
    fn empty_map_is_all_free_with_used_tail() {
        let map = BitMap::empty(10);
        assert_eq!(map.free_count(), 10);
        for index in 0..10 {
            assert!(!map.is_used(index));
        }
        assert!(map.is_used(10));
        assert!(map.is_used(u64::MAX));
        assert_eq!(map.bits[1], 0b1111_1100);
    }

    #[test]
    fn all_used_map_has_nothing_free() {
        let map = BitMap::all_used(10);
        assert_eq!(map.free_count(), 0);
        assert_eq!(map.first_free(0), None);
        assert_eq!(map.bits[1], 0xFF);
    }

    #[test]
    fn set_and_free_toggle_bits() {
        let mut map = BitMap::empty(16);
        map.set_used(3);
        map.set_used(15);
        assert_eq!(map.free_count(), 14);
        assert!(map.is_used(3) && map.is_used(15));
        map.set_free(3);
        assert!(!map.is_used(3));
        assert_eq!(map.free_count(), 15);
    }

    #[test]
    fn out_of_range_mutators_are_noops() {
        let mut map = BitMap::empty(16);
        map.set_used(16);
        map.set_free(16);
        map.use_range(10, 100);
        map.free_range(0, 1000);
        assert_eq!(map.free_count(), 16);
        for index in 0..10 {
            assert!(!map.is_used(index));
        }
        assert_eq!(map.bits, vec![0u8; 2]);
    }

    #[test]
    fn use_and_free_ranges() {
        let mut map = BitMap::empty(64);
        map.use_range(8, 12);
        assert_eq!(map.free_count(), 52);
        for index in 8..20 {
            assert!(map.is_used(index));
        }
        map.free_range(8, 12);
        assert_eq!(map.free_count(), 64);
    }

    #[test]
    fn first_free_respects_hint_and_wraps() {
        let mut map = BitMap::empty(20);
        map.use_range(0, 4);
        assert_eq!(map.first_free(0), Some(4));
        assert_eq!(map.first_free(10), Some(10));
        map.use_range(10, 10);
        assert_eq!(map.first_free(10), Some(4));
        assert_eq!(map.first_free(u64::MAX), Some(4));
        map.use_range(4, 6);
        assert_eq!(map.first_free(0), None);
    }

    #[test]
    fn allocate_one_marks_and_falls_back_around() {
        let mut map = BitMap::empty(8);
        assert_eq!(map.allocate_one(0), Some(0));
        assert_eq!(map.allocate_one(0), Some(1));
        assert_eq!(map.allocate_one(7), Some(7));
        assert_eq!(map.allocate_one(0), Some(2));
        assert!(!map.is_used(6));
    }

    #[test]
    fn allocate_one_exhausted_returns_none() {
        let mut map = BitMap::empty(3);
        assert_eq!(map.allocate_one(0), Some(0));
        assert_eq!(map.allocate_one(0), Some(1));
        assert_eq!(map.allocate_one(0), Some(2));
        assert_eq!(map.allocate_one(0), None);
        assert_eq!(BitMap::empty(0).allocate_one(0), None);
    }

    #[test]
    fn allocate_run_takes_longest_contiguous_prefix() {
        let mut map = BitMap::empty(32);
        map.use_range(0, 5);
        map.use_range(12, 20);
        let run = map.allocate_run(0, 10).unwrap();
        assert_eq!(run, (5, 7));
        assert!((5..12).all(|index| map.is_used(index)));
        assert_eq!(map.free_count(), 0);
    }

    #[test]
    fn allocate_run_never_exceeds_want() {
        let mut map = BitMap::empty(32);
        let run = map.allocate_run(0, 3).unwrap();
        assert_eq!(run, (0, 3));
        let run = map.allocate_run(0, 100).unwrap();
        assert_eq!(run, (3, 29));
        assert_eq!(map.free_count(), 0);
        assert_eq!(map.allocate_run(0, 1), None);
    }

    #[test]
    fn allocate_run_never_spans_the_tail_bits() {
        let mut map = BitMap::empty(10);
        map.free_range(0, 10);
        let run = map.allocate_run(0, 100).unwrap();
        assert_eq!(run, (0, 10));
        assert_eq!(map.free_count(), 0);
    }

    #[test]
    fn allocate_run_starts_at_hint_when_available() {
        let mut map = BitMap::empty(100);
        map.use_range(0, 50);
        let run = map.allocate_run(60, 10).unwrap();
        assert_eq!(run, (60, 10));
    }

    #[test]
    fn store_and_load_roundtrip_across_blocks() {
        let mut original = BitMap::empty(32_768 + 1000);
        original.use_range(0, 10);
        original.use_range(32_770, 30);
        let mut dev = MemoryDevice::new(4);
        original.store(&mut dev, 1).unwrap();
        let loaded = BitMap::load(&mut dev, 1, 2, 32_768 + 1000).unwrap();
        assert_eq!(loaded, original);
    }

    #[test]
    fn load_normalizes_dirty_tail_bits() {
        let mut dev = MemoryDevice::new(1);
        let mut block = [0u8; BLOCK_BYTES];
        block[..]
            .iter_mut()
            .for_each(|byte| *byte = 0x00);
        dev.write_block(0, &block).unwrap();
        let mut loaded = BitMap::load(&mut dev, 0, 1, 10).unwrap();
        assert_eq!(loaded.free_count(), 10);
        assert_eq!(loaded.allocate_run(0, 100).unwrap(), (0, 10));
    }

    #[test]
    fn store_does_not_clobber_beyond_its_bitmap() {
        let mut dev = MemoryDevice::new(2);
        let mut neighbor = [0u8; BLOCK_BYTES];
        neighbor.fill(0xAB);
        dev.write_block(1, &neighbor).unwrap();
        let map = BitMap::all_used(100);
        map.store(&mut dev, 0).unwrap();
        let mut buf = [0u8; BLOCK_BYTES];
        dev.read_block(1, &mut buf).unwrap();
        assert_eq!(buf, neighbor);
    }

    #[test]
    fn load_maps_bit_count_zero_to_nothing() {
        let mut dev = MemoryDevice::new(1);
        let loaded = BitMap::load(&mut dev, 0, 0, 0).unwrap();
        assert_eq!(loaded.free_count(), 0);
        assert_eq!(loaded, BitMap::empty(0));
    }
}
