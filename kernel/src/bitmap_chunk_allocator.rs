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
}
