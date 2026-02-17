use core::alloc::Layout;

use crate::bitmap_chunk_allocator::BitmapChunkAllocator;

const FREE_BLOCK_SIZE: usize = core::mem::size_of::<FreeBlock>();
const FREE_BLOCK_ALIGN: usize = core::mem::align_of::<FreeBlock>();

struct FreeBlock {
    size: usize,
    next: *mut FreeBlock,
}

pub struct FreeListAllocator {
    chunk_allocator: BitmapChunkAllocator,
    free_list: *mut FreeBlock,
}

fn align_up(value: usize, alignment: usize) -> usize {
    (value + alignment - 1) & !(alignment - 1)
}

fn effective_size(layout: Layout) -> usize {
    let min_align = if layout.align() > FREE_BLOCK_ALIGN {
        layout.align()
    } else {
        FREE_BLOCK_ALIGN
    };
    let size = if layout.size() > FREE_BLOCK_SIZE {
        layout.size()
    } else {
        FREE_BLOCK_SIZE
    };
    align_up(size, min_align)
}

impl FreeListAllocator {
    pub fn new(chunk_allocator: BitmapChunkAllocator) -> Self {
        FreeListAllocator {
            chunk_allocator,
            free_list: core::ptr::null_mut(),
        }
    }

    pub fn allocate(&mut self, layout: Layout) -> *mut u8 {
        if layout.size() == 0 {
            return core::ptr::null_mut();
        }

        let size = effective_size(layout);

        if let Some(ptr) = self.find_and_remove(size) {
            return ptr;
        }

        let chunk_layout = Layout::from_size_align(size, layout.align())
            .expect("invalid layout");
        if let Some(allocation) = self.chunk_allocator.allocate(chunk_layout) {
            let total = allocation.chunk_count * allocation.chunk_size;
            // Safety: the chunk allocator returned a valid, writable memory region
            // of `total` bytes at `allocation.ptr`. We place a FreeBlock at its start.
            unsafe {
                let block = allocation.ptr as *mut FreeBlock;
                (*block).size = total;
                (*block).next = self.free_list;
                self.free_list = block;
            }

            if let Some(ptr) = self.find_and_remove(size) {
                return ptr;
            }
        }

        core::ptr::null_mut()
    }

    pub fn deallocate(&mut self, ptr: *mut u8, layout: Layout) {
        if ptr.is_null() || layout.size() == 0 {
            return;
        }
        let size = effective_size(layout);
        // Safety: ptr was previously returned by allocate(), so it points to
        // a region of at least `size` bytes (>= FREE_BLOCK_SIZE) that is no
        // longer in use. We write a FreeBlock header into it.
        unsafe {
            let block = ptr as *mut FreeBlock;
            (*block).size = size;
            (*block).next = self.free_list;
            self.free_list = block;
        }
    }

    fn find_and_remove(&mut self, size: usize) -> Option<*mut u8> {
        let mut prev: *mut FreeBlock = core::ptr::null_mut();
        let mut current = self.free_list;

        // Safety: we only traverse pointers that were either set during chunk
        // acquisition (valid chunk memory) or during a previous split. The loop
        // terminates when current is null.
        unsafe {
            while !current.is_null() {
                if (*current).size >= size {
                    let remainder = (*current).size - size;

                    if remainder >= FREE_BLOCK_SIZE {
                        let new_block = (current as *mut u8).add(size) as *mut FreeBlock;
                        (*new_block).size = remainder;
                        (*new_block).next = (*current).next;

                        if prev.is_null() {
                            self.free_list = new_block;
                        } else {
                            (*prev).next = new_block;
                        }
                    } else {
                        if prev.is_null() {
                            self.free_list = (*current).next;
                        } else {
                            (*prev).next = (*current).next;
                        }
                    }

                    return Some(current as *mut u8);
                }

                prev = current;
                current = (*current).next;
            }
        }

        None
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

    #[test]
    fn two_allocations_do_not_overlap() {
        let mut memory = vec![0u8; 10 * CHUNK_SIZE];
        let base = memory.as_mut_ptr() as usize;
        let chunk_allocator = BitmapChunkAllocator::with_chunk_size(
            CHUNK_SIZE,
            &[(base, memory.len())],
        );

        let mut allocator = FreeListAllocator::new(chunk_allocator);

        let layout = Layout::from_size_align(64, 8).unwrap();
        let first = allocator.allocate(layout);
        let second = allocator.allocate(layout);

        assert!(!first.is_null());
        assert!(!second.is_null());
        assert_ne!(first, second);

        let distance = (second as usize).abs_diff(first as usize);
        assert!(distance >= 64);
    }

    #[test]
    fn allocate_zero_bytes_returns_null() {
        let mut memory = vec![0u8; 10 * CHUNK_SIZE];
        let base = memory.as_mut_ptr() as usize;
        let chunk_allocator = BitmapChunkAllocator::with_chunk_size(
            CHUNK_SIZE,
            &[(base, memory.len())],
        );

        let mut allocator = FreeListAllocator::new(chunk_allocator);

        let layout = Layout::from_size_align(0, 1).unwrap();
        let ptr = allocator.allocate(layout);
        assert!(ptr.is_null());
    }

    #[test]
    fn allocation_pointer_within_chunk_bounds() {
        let mut memory = vec![0u8; 10 * CHUNK_SIZE];
        let base = memory.as_mut_ptr() as usize;
        let end = base + memory.len();
        let chunk_allocator = BitmapChunkAllocator::with_chunk_size(
            CHUNK_SIZE,
            &[(base, memory.len())],
        );

        let mut allocator = FreeListAllocator::new(chunk_allocator);

        let layout = Layout::from_size_align(64, 8).unwrap();
        let ptr = allocator.allocate(layout) as usize;
        assert!(ptr >= base);
        assert!(ptr + 64 <= end);
    }

    #[test]
    fn exhausting_chunk_triggers_second_chunk_request() {
        let mut memory = vec![0u8; 10 * CHUNK_SIZE];
        let base = memory.as_mut_ptr() as usize;
        let chunk_allocator = BitmapChunkAllocator::with_chunk_size(
            CHUNK_SIZE,
            &[(base, memory.len())],
        );

        let mut allocator = FreeListAllocator::new(chunk_allocator);

        let alloc_size = CHUNK_SIZE / 2;
        let layout = Layout::from_size_align(alloc_size, 8).unwrap();

        let first = allocator.allocate(layout);
        let second = allocator.allocate(layout);
        let third = allocator.allocate(layout);

        assert!(!first.is_null());
        assert!(!second.is_null());
        assert!(!third.is_null());

        let all_different = first != second && second != third && first != third;
        assert!(all_different);
    }

    #[test]
    fn deallocate_reuses_freed_memory() {
        let mut memory = vec![0u8; 10 * CHUNK_SIZE];
        let base = memory.as_mut_ptr() as usize;
        let chunk_allocator = BitmapChunkAllocator::with_chunk_size(
            CHUNK_SIZE,
            &[(base, memory.len())],
        );

        let mut allocator = FreeListAllocator::new(chunk_allocator);

        let layout = Layout::from_size_align(128, 8).unwrap();
        let first = allocator.allocate(layout);
        let _second = allocator.allocate(layout);

        allocator.deallocate(first, layout);

        let third = allocator.allocate(layout);
        assert_eq!(third, first);
    }

    #[test]
    fn deallocate_null_is_noop() {
        let mut memory = vec![0u8; 10 * CHUNK_SIZE];
        let base = memory.as_mut_ptr() as usize;
        let chunk_allocator = BitmapChunkAllocator::with_chunk_size(
            CHUNK_SIZE,
            &[(base, memory.len())],
        );

        let mut allocator = FreeListAllocator::new(chunk_allocator);

        let layout = Layout::from_size_align(64, 8).unwrap();
        allocator.deallocate(core::ptr::null_mut(), layout);

        let ptr = allocator.allocate(layout);
        assert!(!ptr.is_null());
    }

    #[test]
    fn multiple_deallocate_reallocate_cycles() {
        let mut memory = vec![0u8; 10 * CHUNK_SIZE];
        let base = memory.as_mut_ptr() as usize;
        let chunk_allocator = BitmapChunkAllocator::with_chunk_size(
            CHUNK_SIZE,
            &[(base, memory.len())],
        );

        let mut allocator = FreeListAllocator::new(chunk_allocator);

        let layout = Layout::from_size_align(256, 8).unwrap();

        for _ in 0..100 {
            let ptr = allocator.allocate(layout);
            assert!(!ptr.is_null());
            allocator.deallocate(ptr, layout);
        }
    }

    #[test]
    fn allocate_returns_null_when_exhausted() {
        let mut memory = vec![0u8; 2 * CHUNK_SIZE];
        let base = memory.as_mut_ptr() as usize;
        let chunk_allocator = BitmapChunkAllocator::with_chunk_size(
            CHUNK_SIZE,
            &[(base, memory.len())],
        );

        let mut allocator = FreeListAllocator::new(chunk_allocator);

        let layout = Layout::from_size_align(CHUNK_SIZE, 8).unwrap();
        let mut count = 0;
        loop {
            let ptr = allocator.allocate(layout);
            if ptr.is_null() {
                break;
            }
            count += 1;
            assert!(count <= 2);
        }
        assert!(count > 0);
    }

    #[test]
    fn allocated_memory_is_writable() {
        let mut memory = vec![0u8; 10 * CHUNK_SIZE];
        let base = memory.as_mut_ptr() as usize;
        let chunk_allocator = BitmapChunkAllocator::with_chunk_size(
            CHUNK_SIZE,
            &[(base, memory.len())],
        );

        let mut allocator = FreeListAllocator::new(chunk_allocator);

        let layout = Layout::from_size_align(256, 8).unwrap();
        let ptr = allocator.allocate(layout);
        assert!(!ptr.is_null());

        // Safety: ptr points to a valid 256-byte allocation from our test memory.
        unsafe {
            core::ptr::write_bytes(ptr, 0xAB, 256);
            let slice = core::slice::from_raw_parts(ptr, 256);
            assert!(slice.iter().all(|&b| b == 0xAB));
        }
    }
}
