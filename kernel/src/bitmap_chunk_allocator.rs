pub const CHUNK_SIZE: usize = 64 * 1024;
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
    bitmap: [u64; MAX_BITMAP_WORDS],
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
}
