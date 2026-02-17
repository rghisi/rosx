use core::alloc::Layout;

use crate::bitmap_chunk_allocator::BitmapChunkAllocator;

struct FreeBlock {
    size: usize,
    next: *mut FreeBlock,
}

pub struct FreeListAllocator {
    chunk_allocator: BitmapChunkAllocator,
    free_list: *mut FreeBlock,
}

impl FreeListAllocator {
    pub fn new(chunk_allocator: BitmapChunkAllocator) -> Self {
        FreeListAllocator {
            chunk_allocator,
            free_list: core::ptr::null_mut(),
        }
    }

    pub fn allocate(&mut self, _layout: Layout) -> *mut u8 {
        core::ptr::null_mut()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::alloc::Layout;
    use crate::bitmap_chunk_allocator::BitmapChunkAllocator;

    const CHUNK_SIZE: usize = 4096;

    #[test]
    fn allocate_small_returns_non_null() {
        let mut memory = vec![0u8; 10 * CHUNK_SIZE];
        let base = memory.as_mut_ptr() as usize;
        let chunk_allocator = BitmapChunkAllocator::with_chunk_size(
            CHUNK_SIZE,
            &[(base, memory.len())],
        );

        let mut allocator = FreeListAllocator::new(chunk_allocator);

        let layout = Layout::from_size_align(64, 8).unwrap();
        let ptr = allocator.allocate(layout);
        assert!(!ptr.is_null());
    }
}
