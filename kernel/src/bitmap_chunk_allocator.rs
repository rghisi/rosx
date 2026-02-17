pub const CHUNK_SIZE: usize = 64 * 1024;
const BITS_PER_WORD: usize = usize::BITS as usize;
const MAX_BITMAP_WORDS: usize = 2048;
const MAX_REGIONS: usize = 32;

struct Region {
    base: usize,
    chunk_count: usize,
    bitmap_offset: usize,
}

pub struct BitmapChunkAllocator {
    regions: [Region; MAX_REGIONS],
    region_count: usize,
    bitmap: [usize; MAX_BITMAP_WORDS],
    total_chunks: usize,
}

impl BitmapChunkAllocator {
    pub fn add_region(&mut self, base: usize, size: usize) {
        let chunk_count = size / CHUNK_SIZE;
        if chunk_count == 0 || self.region_count >= MAX_REGIONS {
            return;
        }
        self.regions[self.region_count] = Region {
            base,
            chunk_count,
            bitmap_offset: self.total_chunks,
        };
        self.region_count += 1;
        self.total_chunks += chunk_count;
    }

    pub fn allocate(&mut self, chunk_count: usize) -> Option<*mut u8> {
        if chunk_count == 0 {
            return None;
        }
        for r in 0..self.region_count {
            let base = self.regions[r].base;
            let region_chunks = self.regions[r].chunk_count;
            let bitmap_offset = self.regions[r].bitmap_offset;
            if region_chunks < chunk_count {
                continue;
            }
            if let Some(start) = self.find_free_run(bitmap_offset, region_chunks, chunk_count) {
                self.mark_bits(start, chunk_count, true);
                let chunk_in_region = start - bitmap_offset;
                let addr = base + chunk_in_region * CHUNK_SIZE;
                return Some(addr as *mut u8);
            }
        }
        None
    }

    pub fn deallocate(&mut self, ptr: *mut u8, chunk_count: usize) {
        let addr = ptr as usize;
        for r in 0..self.region_count {
            let base = self.regions[r].base;
            let region_chunks = self.regions[r].chunk_count;
            let bitmap_offset = self.regions[r].bitmap_offset;
            let region_end = base + region_chunks * CHUNK_SIZE;
            if addr >= base && addr < region_end {
                let chunk_in_region = (addr - base) / CHUNK_SIZE;
                let bit_start = bitmap_offset + chunk_in_region;
                self.mark_bits(bit_start, chunk_count, false);
                return;
            }
        }
    }

    pub fn used_chunks(&self) -> usize {
        let mut count = 0;
        for r in 0..self.region_count {
            let offset = self.regions[r].bitmap_offset;
            let chunks = self.regions[r].chunk_count;
            for i in offset..offset + chunks {
                if self.is_bit_set(i) {
                    count += 1;
                }
            }
        }
        count
    }

    pub fn free_chunks(&self) -> usize {
        self.total_chunks - self.used_chunks()
    }

    fn find_free_run(&self, bitmap_offset: usize, region_chunks: usize, needed: usize) -> Option<usize> {
        let mut run_start = bitmap_offset;
        let region_end = bitmap_offset + region_chunks;
        while run_start + needed <= region_end {
            let mut run_len = 0;
            while run_len < needed {
                let bit_index = run_start + run_len;
                if self.is_bit_set(bit_index) {
                    run_start = bit_index + 1;
                    break;
                }
                run_len += 1;
            }
            if run_len == needed {
                return Some(run_start);
            }
        }
        None
    }

    fn is_bit_set(&self, bit_index: usize) -> bool {
        let word = bit_index / BITS_PER_WORD;
        let bit = bit_index % BITS_PER_WORD;
        self.bitmap[word] & (1usize << bit) != 0
    }

    fn mark_bits(&mut self, start: usize, count: usize, used: bool) {
        for i in start..start + count {
            let word = i / BITS_PER_WORD;
            let bit = i % BITS_PER_WORD;
            if used {
                self.bitmap[word] |= 1usize << bit;
            } else {
                self.bitmap[word] &= !(1usize << bit);
            }
        }
    }

    pub fn new() -> Self {
        const EMPTY_REGION: Region = Region {
            base: 0,
            chunk_count: 0,
            bitmap_offset: 0,
        };
        BitmapChunkAllocator {
            regions: [EMPTY_REGION; MAX_REGIONS],
            region_count: 0,
            bitmap: [0; MAX_BITMAP_WORDS],
            total_chunks: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_allocator_has_zero_regions() {
        let allocator = BitmapChunkAllocator::new();
        assert_eq!(allocator.region_count, 0);
        assert_eq!(allocator.total_chunks, 0);
    }

    #[test]
    fn add_region_with_exact_chunk_alignment() {
        let mut allocator = BitmapChunkAllocator::new();
        let base = 0x10_0000;
        let size = 4 * CHUNK_SIZE;

        allocator.add_region(base, size);

        assert_eq!(allocator.region_count, 1);
        assert_eq!(allocator.total_chunks, 4);
        assert_eq!(allocator.regions[0].base, base);
        assert_eq!(allocator.regions[0].chunk_count, 4);
        assert_eq!(allocator.regions[0].bitmap_offset, 0);
    }

    #[test]
    fn add_region_trims_unaligned_tail() {
        let mut allocator = BitmapChunkAllocator::new();
        let base = 0x10_0000;
        let size = 3 * CHUNK_SIZE + 100;

        allocator.add_region(base, size);

        assert_eq!(allocator.total_chunks, 3);
        assert_eq!(allocator.regions[0].chunk_count, 3);
    }

    #[test]
    fn add_multiple_regions_assigns_sequential_bitmap_offsets() {
        let mut allocator = BitmapChunkAllocator::new();

        allocator.add_region(0x10_0000, 2 * CHUNK_SIZE);
        allocator.add_region(0x80_0000, 5 * CHUNK_SIZE);

        assert_eq!(allocator.region_count, 2);
        assert_eq!(allocator.total_chunks, 7);
        assert_eq!(allocator.regions[0].bitmap_offset, 0);
        assert_eq!(allocator.regions[1].bitmap_offset, 2);
    }

    #[test]
    fn add_region_smaller_than_one_chunk_is_ignored() {
        let mut allocator = BitmapChunkAllocator::new();

        allocator.add_region(0x10_0000, CHUNK_SIZE - 1);

        assert_eq!(allocator.region_count, 0);
        assert_eq!(allocator.total_chunks, 0);
    }

    #[test]
    fn allocate_single_chunk() {
        let mut allocator = BitmapChunkAllocator::new();
        let base = 0x10_0000;
        allocator.add_region(base, 4 * CHUNK_SIZE);

        let ptr = allocator.allocate(1);

        assert_eq!(ptr, Some(base as *mut u8));
    }

    #[test]
    fn allocate_multiple_contiguous_chunks() {
        let mut allocator = BitmapChunkAllocator::new();
        let base = 0x10_0000;
        allocator.add_region(base, 8 * CHUNK_SIZE);

        let ptr = allocator.allocate(3);

        assert_eq!(ptr, Some(base as *mut u8));
    }

    #[test]
    fn allocate_successive_calls_return_non_overlapping_regions() {
        let mut allocator = BitmapChunkAllocator::new();
        let base = 0x10_0000;
        allocator.add_region(base, 8 * CHUNK_SIZE);

        let first = allocator.allocate(2).unwrap();
        let second = allocator.allocate(3).unwrap();

        assert_eq!(first, base as *mut u8);
        assert_eq!(second, (base + 2 * CHUNK_SIZE) as *mut u8);
    }

    #[test]
    fn allocate_returns_none_when_not_enough_contiguous_chunks() {
        let mut allocator = BitmapChunkAllocator::new();
        allocator.add_region(0x10_0000, 2 * CHUNK_SIZE);

        let ptr = allocator.allocate(3);

        assert_eq!(ptr, None);
    }

    #[test]
    fn allocate_returns_none_on_empty_allocator() {
        let mut allocator = BitmapChunkAllocator::new();

        let ptr = allocator.allocate(1);

        assert_eq!(ptr, None);
    }

    #[test]
    fn allocate_zero_chunks_returns_none() {
        let mut allocator = BitmapChunkAllocator::new();
        allocator.add_region(0x10_0000, 4 * CHUNK_SIZE);

        let ptr = allocator.allocate(0);

        assert_eq!(ptr, None);
    }

    #[test]
    fn allocate_until_full_then_returns_none() {
        let mut allocator = BitmapChunkAllocator::new();
        allocator.add_region(0x10_0000, 2 * CHUNK_SIZE);

        let first = allocator.allocate(1);
        let second = allocator.allocate(1);
        let third = allocator.allocate(1);

        assert!(first.is_some());
        assert!(second.is_some());
        assert_eq!(third, None);
    }

    #[test]
    fn allocate_from_second_region_when_first_is_full() {
        let mut allocator = BitmapChunkAllocator::new();
        allocator.add_region(0x10_0000, 1 * CHUNK_SIZE);
        allocator.add_region(0x80_0000, 4 * CHUNK_SIZE);

        let first = allocator.allocate(1).unwrap();
        let second = allocator.allocate(2).unwrap();

        assert_eq!(first, 0x10_0000 as *mut u8);
        assert_eq!(second, 0x80_0000 as *mut u8);
    }

    #[test]
    fn deallocate_then_reallocate_same_space() {
        let mut allocator = BitmapChunkAllocator::new();
        let base = 0x10_0000;
        allocator.add_region(base, 2 * CHUNK_SIZE);

        allocator.allocate(2).unwrap();
        assert_eq!(allocator.allocate(1), None);

        allocator.deallocate(base as *mut u8, 2);

        let ptr = allocator.allocate(2);
        assert_eq!(ptr, Some(base as *mut u8));
    }

    #[test]
    fn deallocate_creates_gap_for_new_contiguous_allocation() {
        let mut allocator = BitmapChunkAllocator::new();
        let base = 0x10_0000;
        allocator.add_region(base, 4 * CHUNK_SIZE);

        let a = allocator.allocate(1).unwrap();
        let _b = allocator.allocate(1).unwrap();
        let _c = allocator.allocate(1).unwrap();
        allocator.allocate(1).unwrap();

        allocator.deallocate(a, 1);
        let reused = allocator.allocate(1).unwrap();
        assert_eq!(reused, base as *mut u8);
    }

    #[test]
    fn deallocate_middle_chunks_allows_reuse() {
        let mut allocator = BitmapChunkAllocator::new();
        let base = 0x10_0000;
        allocator.add_region(base, 4 * CHUNK_SIZE);

        allocator.allocate(1).unwrap();
        let b = allocator.allocate(2).unwrap();
        allocator.allocate(1).unwrap();

        allocator.deallocate(b, 2);

        let reused = allocator.allocate(2).unwrap();
        assert_eq!(reused, (base + CHUNK_SIZE) as *mut u8);
    }

    #[test]
    fn empty_allocator_has_zero_free_and_used() {
        let allocator = BitmapChunkAllocator::new();
        assert_eq!(allocator.free_chunks(), 0);
        assert_eq!(allocator.used_chunks(), 0);
    }

    #[test]
    fn fresh_region_all_chunks_free() {
        let mut allocator = BitmapChunkAllocator::new();
        allocator.add_region(0x10_0000, 4 * CHUNK_SIZE);

        assert_eq!(allocator.free_chunks(), 4);
        assert_eq!(allocator.used_chunks(), 0);
    }

    #[test]
    fn counts_reflect_allocations() {
        let mut allocator = BitmapChunkAllocator::new();
        allocator.add_region(0x10_0000, 4 * CHUNK_SIZE);

        allocator.allocate(1);
        assert_eq!(allocator.used_chunks(), 1);
        assert_eq!(allocator.free_chunks(), 3);

        allocator.allocate(2);
        assert_eq!(allocator.used_chunks(), 3);
        assert_eq!(allocator.free_chunks(), 1);
    }

    #[test]
    fn counts_reflect_deallocations() {
        let mut allocator = BitmapChunkAllocator::new();
        let base = 0x10_0000;
        allocator.add_region(base, 4 * CHUNK_SIZE);

        let ptr = allocator.allocate(3).unwrap();
        assert_eq!(allocator.used_chunks(), 3);

        allocator.deallocate(ptr, 2);
        assert_eq!(allocator.used_chunks(), 1);
        assert_eq!(allocator.free_chunks(), 3);
    }

    #[test]
    fn bits_per_word_matches_native_word_size() {
        assert_eq!(BITS_PER_WORD, usize::BITS as usize);
    }

    #[test]
    fn counts_across_multiple_regions() {
        let mut allocator = BitmapChunkAllocator::new();
        allocator.add_region(0x10_0000, 2 * CHUNK_SIZE);
        allocator.add_region(0x80_0000, 3 * CHUNK_SIZE);

        assert_eq!(allocator.free_chunks(), 5);
        allocator.allocate(1);
        allocator.allocate(2);
        assert_eq!(allocator.used_chunks(), 3);
        assert_eq!(allocator.free_chunks(), 2);
    }
}
