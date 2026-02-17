pub const DEFAULT_CHUNK_SIZE: usize = 64 * 1024;
const BITS_PER_WORD: usize = usize::BITS as usize;
const METADATA_ALIGNMENT: usize = 16;

pub struct Allocation {
    pub ptr: *mut u8,
    pub chunk_count: usize,
    pub chunk_size: usize,
}

struct Region {
    base: usize,
    chunk_count: usize,
    bitmap_offset: usize,
}

pub struct BitmapChunkAllocator {
    bitmap: *mut usize,
    #[allow(dead_code)]
    bitmap_len: usize,
    regions: *mut Region,
    region_count: usize,
    total_chunks: usize,
    chunk_size: usize,
}

fn align_up(value: usize, alignment: usize) -> usize {
    (value + alignment - 1) & !(alignment - 1)
}

impl BitmapChunkAllocator {
    pub fn new(ranges: &[(usize, usize)]) -> Self {
        Self::with_chunk_size(DEFAULT_CHUNK_SIZE, ranges)
    }

    pub fn with_chunk_size(chunk_size: usize, ranges: &[(usize, usize)]) -> Self {
        assert!(!ranges.is_empty(), "at least one range required");
        let region_count = ranges.len();

        let raw_total_chunks: usize = ranges.iter()
            .take(region_count)
            .map(|&(_, size)| size / chunk_size)
            .sum();

        let bitmap_words = (raw_total_chunks + BITS_PER_WORD - 1) / BITS_PER_WORD;
        let regions_bytes = region_count * core::mem::size_of::<Region>();
        let bitmap_bytes = bitmap_words * core::mem::size_of::<usize>();
        let aligned_metadata = align_up(regions_bytes + bitmap_bytes, METADATA_ALIGNMENT);

        let (first_base, first_size) = ranges[0];
        assert!(
            aligned_metadata + chunk_size <= first_size,
            "first range too small for metadata"
        );

        let regions_ptr = first_base as *mut Region;
        let bitmap_ptr = (first_base + regions_bytes) as *mut usize;

        // Safety: first_base points to writable memory large enough for aligned_metadata bytes.
        // The caller guarantees this by providing a valid memory range.
        unsafe {
            core::ptr::write_bytes(first_base as *mut u8, 0, aligned_metadata);
        }

        let usable_start = first_base + aligned_metadata;
        let usable_size = first_size - aligned_metadata;
        let first_chunk_count = usable_size / chunk_size;

        let mut total_chunks = first_chunk_count;
        let mut bitmap_offset = first_chunk_count;

        // Safety: regions_ptr points into the zero-initialized metadata area
        // with space for region_count Region entries.
        unsafe {
            *regions_ptr = Region {
                base: usable_start,
                chunk_count: first_chunk_count,
                bitmap_offset: 0,
            };

            for i in 1..region_count {
                let (base, size) = ranges[i];
                let chunk_count = size / chunk_size;
                *regions_ptr.add(i) = Region {
                    base,
                    chunk_count,
                    bitmap_offset,
                };
                bitmap_offset += chunk_count;
                total_chunks += chunk_count;
            }
        }

        BitmapChunkAllocator {
            bitmap: bitmap_ptr,
            bitmap_len: bitmap_words,
            regions: regions_ptr,
            region_count,
            total_chunks,
            chunk_size,
        }
    }

    pub fn allocate(&mut self, bytes: usize) -> Option<Allocation> {
        if bytes == 0 {
            return None;
        }
        let chunk_count = (bytes + self.chunk_size - 1) / self.chunk_size;
        for r in 0..self.region_count {
            let region = self.region(r);
            let base = region.base;
            let region_chunks = region.chunk_count;
            let bitmap_offset = region.bitmap_offset;
            if region_chunks < chunk_count {
                continue;
            }
            if let Some(start) = self.find_free_run(bitmap_offset, region_chunks, chunk_count) {
                self.mark_bits(start, chunk_count, true);
                let chunk_in_region = start - bitmap_offset;
                let addr = base + chunk_in_region * self.chunk_size;
                return Some(Allocation {
                    ptr: addr as *mut u8,
                    chunk_count,
                    chunk_size: self.chunk_size,
                });
            }
        }
        None
    }

    pub fn deallocate(&mut self, ptr: *mut u8, chunk_count: usize) {
        let addr = ptr as usize;
        for r in 0..self.region_count {
            let region = self.region(r);
            let base = region.base;
            let region_chunks = region.chunk_count;
            let bitmap_offset = region.bitmap_offset;
            let region_end = base + region_chunks * self.chunk_size;
            if addr >= base && addr < region_end {
                let chunk_in_region = (addr - base) / self.chunk_size;
                let bit_start = bitmap_offset + chunk_in_region;
                self.mark_bits(bit_start, chunk_count, false);
                return;
            }
        }
    }

    pub fn used_chunks(&self) -> usize {
        let mut count = 0;
        for r in 0..self.region_count {
            let region = self.region(r);
            let offset = region.bitmap_offset;
            let chunks = region.chunk_count;
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

    fn region(&self, index: usize) -> &Region {
        // Safety: index is always < self.region_count, which was bounded
        // by the number of regions written during construction.
        unsafe { &*self.regions.add(index) }
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
        // Safety: bit_index is bounded by total_chunks, which fits within bitmap_len words.
        unsafe { *self.bitmap.add(word) & (1usize << bit) != 0 }
    }

    fn mark_bits(&mut self, start: usize, count: usize, used: bool) {
        for i in start..start + count {
            let word = i / BITS_PER_WORD;
            let bit = i % BITS_PER_WORD;
            // Safety: bit indices are bounded by total_chunks, fitting within bitmap_len words.
            unsafe {
                if used {
                    *self.bitmap.add(word) |= 1usize << bit;
                } else {
                    *self.bitmap.add(word) &= !(1usize << bit);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn metadata_overhead(region_count: usize, raw_total_chunks: usize) -> usize {
        let bitmap_words = (raw_total_chunks + BITS_PER_WORD - 1) / BITS_PER_WORD;
        let regions_bytes = region_count * core::mem::size_of::<Region>();
        let bitmap_bytes = bitmap_words * core::mem::size_of::<usize>();
        align_up(regions_bytes + bitmap_bytes, METADATA_ALIGNMENT)
    }

    fn usable_base(base: usize, region_count: usize, raw_total_chunks: usize) -> usize {
        base + metadata_overhead(region_count, raw_total_chunks)
    }

    #[test]
    fn bits_per_word_matches_native_word_size() {
        assert_eq!(BITS_PER_WORD, usize::BITS as usize);
    }

    #[test]
    fn new_embeds_metadata_in_first_range() {
        let mut memory = vec![0u8; 8 * DEFAULT_CHUNK_SIZE];
        let base = memory.as_mut_ptr() as usize;

        let allocator = BitmapChunkAllocator::new(&[(base, memory.len())]);

        let overhead = metadata_overhead(1, 8);
        let expected_chunks = (8 * DEFAULT_CHUNK_SIZE - overhead) / DEFAULT_CHUNK_SIZE;
        assert_eq!(allocator.free_chunks(), expected_chunks);
        assert_eq!(allocator.used_chunks(), 0);
    }

    #[test]
    fn new_trims_unaligned_tail() {
        let mut memory = vec![0u8; 4 * DEFAULT_CHUNK_SIZE + 100];
        let base = memory.as_mut_ptr() as usize;

        let allocator = BitmapChunkAllocator::new(&[(base, memory.len())]);

        let overhead = metadata_overhead(1, 4);
        let expected_chunks = (4 * DEFAULT_CHUNK_SIZE + 100 - overhead) / DEFAULT_CHUNK_SIZE;
        assert_eq!(allocator.free_chunks(), expected_chunks);
    }

    #[test]
    fn new_with_multiple_ranges() {
        let mut mem1 = vec![0u8; 4 * DEFAULT_CHUNK_SIZE];
        let mut mem2 = vec![0u8; 5 * DEFAULT_CHUNK_SIZE];
        let base1 = mem1.as_mut_ptr() as usize;
        let base2 = mem2.as_mut_ptr() as usize;

        let allocator = BitmapChunkAllocator::new(&[
            (base1, mem1.len()),
            (base2, mem2.len()),
        ]);

        let overhead = metadata_overhead(2, 9);
        let first_region_chunks = (4 * DEFAULT_CHUNK_SIZE - overhead) / DEFAULT_CHUNK_SIZE;
        let expected_total = first_region_chunks + 5;
        assert_eq!(allocator.region_count, 2);
        assert_eq!(allocator.free_chunks(), expected_total);
    }

    #[test]
    fn allocate_single_chunk() {
        let mut memory = vec![0u8; 4 * DEFAULT_CHUNK_SIZE];
        let base = memory.as_mut_ptr() as usize;
        let usable = usable_base(base, 1, 4);

        let mut allocator = BitmapChunkAllocator::new(&[(base, memory.len())]);

        let alloc = allocator.allocate(DEFAULT_CHUNK_SIZE).unwrap();

        assert_eq!(alloc.ptr, usable as *mut u8);
    }

    #[test]
    fn allocate_multiple_contiguous_chunks() {
        let mut memory = vec![0u8; 8 * DEFAULT_CHUNK_SIZE];
        let base = memory.as_mut_ptr() as usize;
        let usable = usable_base(base, 1, 8);

        let mut allocator = BitmapChunkAllocator::new(&[(base, memory.len())]);

        let alloc = allocator.allocate(3 * DEFAULT_CHUNK_SIZE).unwrap();

        assert_eq!(alloc.ptr, usable as *mut u8);
    }

    #[test]
    fn allocate_successive_calls_return_non_overlapping_regions() {
        let mut memory = vec![0u8; 8 * DEFAULT_CHUNK_SIZE];
        let base = memory.as_mut_ptr() as usize;
        let usable = usable_base(base, 1, 8);

        let mut allocator = BitmapChunkAllocator::new(&[(base, memory.len())]);

        let first = allocator.allocate(2 * DEFAULT_CHUNK_SIZE).unwrap();
        let second = allocator.allocate(3 * DEFAULT_CHUNK_SIZE).unwrap();

        assert_eq!(first.ptr, usable as *mut u8);
        assert_eq!(second.ptr, (usable + 2 * DEFAULT_CHUNK_SIZE) as *mut u8);
    }

    #[test]
    fn allocate_returns_none_when_not_enough_contiguous_chunks() {
        let mut memory = vec![0u8; 3 * DEFAULT_CHUNK_SIZE];
        let base = memory.as_mut_ptr() as usize;

        let mut allocator = BitmapChunkAllocator::new(&[(base, memory.len())]);

        let overhead = metadata_overhead(1, 3);
        let available = (3 * DEFAULT_CHUNK_SIZE - overhead) / DEFAULT_CHUNK_SIZE;
        let result = allocator.allocate((available + 1) * DEFAULT_CHUNK_SIZE);

        assert!(result.is_none());
    }

    #[test]
    fn allocate_zero_bytes_returns_none_default_chunk() {
        let mut memory = vec![0u8; 4 * DEFAULT_CHUNK_SIZE];
        let base = memory.as_mut_ptr() as usize;

        let mut allocator = BitmapChunkAllocator::new(&[(base, memory.len())]);

        assert!(allocator.allocate(0).is_none());
    }

    #[test]
    fn allocate_until_full_then_returns_none() {
        let mut memory = vec![0u8; 3 * DEFAULT_CHUNK_SIZE];
        let base = memory.as_mut_ptr() as usize;

        let mut allocator = BitmapChunkAllocator::new(&[(base, memory.len())]);

        let overhead = metadata_overhead(1, 3);
        let available = (3 * DEFAULT_CHUNK_SIZE - overhead) / DEFAULT_CHUNK_SIZE;

        for _ in 0..available {
            assert!(allocator.allocate(DEFAULT_CHUNK_SIZE).is_some());
        }
        assert!(allocator.allocate(DEFAULT_CHUNK_SIZE).is_none());
    }

    #[test]
    fn allocate_from_second_region_when_first_is_full() {
        let mut mem1 = vec![0u8; 2 * DEFAULT_CHUNK_SIZE];
        let mut mem2 = vec![0u8; 4 * DEFAULT_CHUNK_SIZE];
        let base1 = mem1.as_mut_ptr() as usize;
        let base2 = mem2.as_mut_ptr() as usize;

        let mut allocator = BitmapChunkAllocator::new(&[
            (base1, mem1.len()),
            (base2, mem2.len()),
        ]);

        let overhead = metadata_overhead(2, 6);
        let first_region_chunks = (2 * DEFAULT_CHUNK_SIZE - overhead) / DEFAULT_CHUNK_SIZE;

        for _ in 0..first_region_chunks {
            allocator.allocate(DEFAULT_CHUNK_SIZE).unwrap();
        }

        let from_second = allocator.allocate(2 * DEFAULT_CHUNK_SIZE).unwrap();
        assert_eq!(from_second.ptr, base2 as *mut u8);
    }

    #[test]
    fn deallocate_then_reallocate_same_space() {
        let mut memory = vec![0u8; 3 * DEFAULT_CHUNK_SIZE];
        let base = memory.as_mut_ptr() as usize;
        let usable = usable_base(base, 1, 3);

        let mut allocator = BitmapChunkAllocator::new(&[(base, memory.len())]);

        let overhead = metadata_overhead(1, 3);
        let available = (3 * DEFAULT_CHUNK_SIZE - overhead) / DEFAULT_CHUNK_SIZE;

        let alloc = allocator.allocate(available * DEFAULT_CHUNK_SIZE).unwrap();
        assert!(allocator.allocate(DEFAULT_CHUNK_SIZE).is_none());

        allocator.deallocate(alloc.ptr, available);

        let alloc2 = allocator.allocate(available * DEFAULT_CHUNK_SIZE).unwrap();
        assert_eq!(alloc2.ptr, usable as *mut u8);
    }

    #[test]
    fn deallocate_creates_gap_for_new_contiguous_allocation() {
        let mut memory = vec![0u8; 5 * DEFAULT_CHUNK_SIZE];
        let base = memory.as_mut_ptr() as usize;
        let usable = usable_base(base, 1, 5);

        let mut allocator = BitmapChunkAllocator::new(&[(base, memory.len())]);

        let a = allocator.allocate(DEFAULT_CHUNK_SIZE).unwrap();
        let _b = allocator.allocate(DEFAULT_CHUNK_SIZE).unwrap();
        let _c = allocator.allocate(DEFAULT_CHUNK_SIZE).unwrap();
        allocator.allocate(DEFAULT_CHUNK_SIZE).unwrap();

        allocator.deallocate(a.ptr, 1);
        let reused = allocator.allocate(DEFAULT_CHUNK_SIZE).unwrap();
        assert_eq!(reused.ptr, usable as *mut u8);
    }

    #[test]
    fn deallocate_middle_chunks_allows_reuse() {
        let mut memory = vec![0u8; 5 * DEFAULT_CHUNK_SIZE];
        let base = memory.as_mut_ptr() as usize;
        let usable = usable_base(base, 1, 5);

        let mut allocator = BitmapChunkAllocator::new(&[(base, memory.len())]);

        allocator.allocate(DEFAULT_CHUNK_SIZE).unwrap();
        let b = allocator.allocate(2 * DEFAULT_CHUNK_SIZE).unwrap();
        allocator.allocate(DEFAULT_CHUNK_SIZE).unwrap();

        allocator.deallocate(b.ptr, 2);

        let reused = allocator.allocate(2 * DEFAULT_CHUNK_SIZE).unwrap();
        assert_eq!(reused.ptr, (usable + DEFAULT_CHUNK_SIZE) as *mut u8);
    }

    #[test]
    fn fresh_region_all_chunks_free() {
        let mut memory = vec![0u8; 5 * DEFAULT_CHUNK_SIZE];
        let base = memory.as_mut_ptr() as usize;

        let allocator = BitmapChunkAllocator::new(&[(base, memory.len())]);

        let overhead = metadata_overhead(1, 5);
        let expected = (5 * DEFAULT_CHUNK_SIZE - overhead) / DEFAULT_CHUNK_SIZE;
        assert_eq!(allocator.free_chunks(), expected);
        assert_eq!(allocator.used_chunks(), 0);
    }

    #[test]
    fn counts_reflect_allocations() {
        let mut memory = vec![0u8; 5 * DEFAULT_CHUNK_SIZE];
        let base = memory.as_mut_ptr() as usize;

        let mut allocator = BitmapChunkAllocator::new(&[(base, memory.len())]);

        let total = allocator.free_chunks();

        allocator.allocate(DEFAULT_CHUNK_SIZE);
        assert_eq!(allocator.used_chunks(), 1);
        assert_eq!(allocator.free_chunks(), total - 1);

        allocator.allocate(2 * DEFAULT_CHUNK_SIZE);
        assert_eq!(allocator.used_chunks(), 3);
        assert_eq!(allocator.free_chunks(), total - 3);
    }

    #[test]
    fn counts_reflect_deallocations() {
        let mut memory = vec![0u8; 5 * DEFAULT_CHUNK_SIZE];
        let base = memory.as_mut_ptr() as usize;

        let mut allocator = BitmapChunkAllocator::new(&[(base, memory.len())]);

        let total = allocator.free_chunks();

        let alloc = allocator.allocate(3 * DEFAULT_CHUNK_SIZE).unwrap();
        assert_eq!(allocator.used_chunks(), 3);

        allocator.deallocate(alloc.ptr, 2);
        assert_eq!(allocator.used_chunks(), 1);
        assert_eq!(allocator.free_chunks(), total - 1);
    }

    #[test]
    fn counts_across_multiple_regions() {
        let mut mem1 = vec![0u8; 4 * DEFAULT_CHUNK_SIZE];
        let mut mem2 = vec![0u8; 3 * DEFAULT_CHUNK_SIZE];
        let base1 = mem1.as_mut_ptr() as usize;
        let base2 = mem2.as_mut_ptr() as usize;

        let mut allocator = BitmapChunkAllocator::new(&[
            (base1, mem1.len()),
            (base2, mem2.len()),
        ]);

        let total = allocator.free_chunks();
        allocator.allocate(DEFAULT_CHUNK_SIZE);
        allocator.allocate(2 * DEFAULT_CHUNK_SIZE);
        assert_eq!(allocator.used_chunks(), 3);
        assert_eq!(allocator.free_chunks(), total - 3);
    }

    #[test]
    fn new_accepts_more_than_32_ranges() {
        let range_count = 40;
        let mems: Vec<Vec<u8>> = (0..range_count)
            .map(|_| vec![0u8; 2 * DEFAULT_CHUNK_SIZE])
            .collect();
        let ranges: Vec<(usize, usize)> = mems.iter()
            .map(|m| (m.as_ptr() as usize, m.len()))
            .collect();

        let allocator = BitmapChunkAllocator::new(&ranges);

        assert_eq!(allocator.region_count, range_count);
    }

    #[test]
    fn allocate_bytes_returns_allocation_struct() {
        let chunk_size: usize = 4096;
        let mut memory = vec![0u8; 10 * chunk_size];
        let base = memory.as_mut_ptr() as usize;

        let mut allocator = BitmapChunkAllocator::with_chunk_size(
            chunk_size,
            &[(base, memory.len())],
        );

        let alloc = allocator.allocate(chunk_size + 1).unwrap();

        assert_eq!(alloc.chunk_count, 2);
        assert_eq!(alloc.chunk_size, chunk_size);
        assert!(!alloc.ptr.is_null());
    }

    #[test]
    fn allocate_exact_chunk_boundary() {
        let chunk_size: usize = 4096;
        let mut memory = vec![0u8; 10 * chunk_size];
        let base = memory.as_mut_ptr() as usize;

        let mut allocator = BitmapChunkAllocator::with_chunk_size(
            chunk_size,
            &[(base, memory.len())],
        );

        let alloc = allocator.allocate(chunk_size).unwrap();
        assert_eq!(alloc.chunk_count, 1);

        let alloc2 = allocator.allocate(2 * chunk_size).unwrap();
        assert_eq!(alloc2.chunk_count, 2);
    }

    #[test]
    fn allocate_zero_bytes_returns_none() {
        let chunk_size: usize = 4096;
        let mut memory = vec![0u8; 10 * chunk_size];
        let base = memory.as_mut_ptr() as usize;

        let mut allocator = BitmapChunkAllocator::with_chunk_size(
            chunk_size,
            &[(base, memory.len())],
        );

        assert!(allocator.allocate(0).is_none());
    }

    #[test]
    fn with_custom_chunk_size() {
        let custom_size: usize = 4096;
        let mut memory = vec![0u8; 8 * custom_size];
        let base = memory.as_mut_ptr() as usize;

        let allocator = BitmapChunkAllocator::with_chunk_size(
            custom_size,
            &[(base, memory.len())],
        );

        assert_eq!(allocator.chunk_size, custom_size);
        assert!(allocator.free_chunks() > 0);
        assert!(allocator.free_chunks() < 8);
    }
}
