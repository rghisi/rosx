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

        let align = layout.align();

        if let Some(ptr) = self.find_and_remove(size, align) {
            return ptr;
        }

        let chunk_layout = Layout::from_size_align(size, align)
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

            if let Some(ptr) = self.find_and_remove(size, align) {
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

    fn find_and_remove(&mut self, size: usize, align: usize) -> Option<*mut u8> {
        let mut prev: *mut FreeBlock = core::ptr::null_mut();
        let mut current = self.free_list;

        // Safety: we only traverse pointers that were either set during chunk
        // acquisition (valid chunk memory) or during a previous split. The loop
        // terminates when current is null.
        unsafe {
            while !current.is_null() {
                let block_addr = current as usize;
                let aligned_addr = align_up(block_addr, align);
                let front_padding = aligned_addr - block_addr;

                if front_padding > 0 && front_padding < FREE_BLOCK_SIZE {
                    prev = current;
                    current = (*current).next;
                    continue;
                }

                let needed = front_padding + size;

                if (*current).size >= needed {
                    let tail_remainder = (*current).size - needed;
                    let next = (*current).next;

                    if front_padding >= FREE_BLOCK_SIZE {
                        (*current).size = front_padding;
                        if tail_remainder >= FREE_BLOCK_SIZE {
                            let tail = (aligned_addr + size) as *mut FreeBlock;
                            (*tail).size = tail_remainder;
                            (*tail).next = next;
                            (*current).next = tail;
                        } else {
                            (*current).next = next;
                        }
                    } else {
                        if tail_remainder >= FREE_BLOCK_SIZE {
                            let tail = (aligned_addr + size) as *mut FreeBlock;
                            (*tail).size = tail_remainder;
                            (*tail).next = next;
                            if prev.is_null() {
                                self.free_list = tail;
                            } else {
                                (*prev).next = tail;
                            }
                        } else {
                            if prev.is_null() {
                                self.free_list = next;
                            } else {
                                (*prev).next = next;
                            }
                        }
                    }

                    return Some(aligned_addr as *mut u8);
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

    #[test]
    fn allocate_tiny_size_rounds_up_to_free_block_size() {
        let mut memory = vec![0u8; 10 * CHUNK_SIZE];
        let base = memory.as_mut_ptr() as usize;
        let chunk_allocator = BitmapChunkAllocator::with_chunk_size(
            CHUNK_SIZE,
            &[(base, memory.len())],
        );

        let mut allocator = FreeListAllocator::new(chunk_allocator);

        let layout = Layout::from_size_align(1, 1).unwrap();
        let first = allocator.allocate(layout);
        let second = allocator.allocate(layout);

        assert!(!first.is_null());
        assert!(!second.is_null());

        let distance = (second as usize).abs_diff(first as usize);
        assert!(distance >= FREE_BLOCK_SIZE);
    }

    #[test]
    fn allocate_exactly_free_block_size() {
        let mut memory = vec![0u8; 10 * CHUNK_SIZE];
        let base = memory.as_mut_ptr() as usize;
        let chunk_allocator = BitmapChunkAllocator::with_chunk_size(
            CHUNK_SIZE,
            &[(base, memory.len())],
        );

        let mut allocator = FreeListAllocator::new(chunk_allocator);

        let layout = Layout::from_size_align(FREE_BLOCK_SIZE, FREE_BLOCK_ALIGN).unwrap();
        let ptr = allocator.allocate(layout);
        assert!(!ptr.is_null());

        allocator.deallocate(ptr, layout);
        let reused = allocator.allocate(layout);
        assert_eq!(reused, ptr);
    }

    #[test]
    fn allocate_larger_than_one_chunk() {
        let mut memory = vec![0u8; 10 * CHUNK_SIZE];
        let base = memory.as_mut_ptr() as usize;
        let chunk_allocator = BitmapChunkAllocator::with_chunk_size(
            CHUNK_SIZE,
            &[(base, memory.len())],
        );

        let mut allocator = FreeListAllocator::new(chunk_allocator);

        let big_size = 3 * CHUNK_SIZE;
        let layout = Layout::from_size_align(big_size, 8).unwrap();
        let ptr = allocator.allocate(layout);
        assert!(!ptr.is_null());

        // Safety: ptr points to a valid region of big_size bytes.
        unsafe {
            core::ptr::write_bytes(ptr, 0xCD, big_size);
            let slice = core::slice::from_raw_parts(ptr, big_size);
            assert!(slice.iter().all(|&b| b == 0xCD));
        }
    }

    #[test]
    fn returned_pointer_respects_alignment() {
        let mut memory = vec![0u8; 10 * CHUNK_SIZE];
        let base = memory.as_mut_ptr() as usize;
        let chunk_allocator = BitmapChunkAllocator::with_chunk_size(
            CHUNK_SIZE,
            &[(base, memory.len())],
        );

        let mut allocator = FreeListAllocator::new(chunk_allocator);

        for align_shift in 0..=8 {
            let align = 1usize << align_shift;
            let layout = Layout::from_size_align(64, align).unwrap();
            let ptr = allocator.allocate(layout);
            assert!(!ptr.is_null());
            assert_eq!(
                (ptr as usize) % align,
                0,
                "pointer {:#x} not aligned to {}",
                ptr as usize,
                align
            );
        }
    }

    #[test]
    fn interleaved_alloc_dealloc_fills_gap() {
        let mut memory = vec![0u8; 10 * CHUNK_SIZE];
        let base = memory.as_mut_ptr() as usize;
        let chunk_allocator = BitmapChunkAllocator::with_chunk_size(
            CHUNK_SIZE,
            &[(base, memory.len())],
        );

        let mut allocator = FreeListAllocator::new(chunk_allocator);

        let layout = Layout::from_size_align(128, 8).unwrap();
        let a = allocator.allocate(layout);
        let b = allocator.allocate(layout);
        let c = allocator.allocate(layout);
        assert!(!a.is_null());
        assert!(!b.is_null());
        assert!(!c.is_null());

        allocator.deallocate(b, layout);

        let d = allocator.allocate(layout);
        assert_eq!(d, b);
    }

    #[test]
    fn interleaved_different_sizes_reuses_fitting_gap() {
        let mut memory = vec![0u8; 10 * CHUNK_SIZE];
        let base = memory.as_mut_ptr() as usize;
        let chunk_allocator = BitmapChunkAllocator::with_chunk_size(
            CHUNK_SIZE,
            &[(base, memory.len())],
        );

        let mut allocator = FreeListAllocator::new(chunk_allocator);

        let big_layout = Layout::from_size_align(512, 8).unwrap();
        let small_layout = Layout::from_size_align(64, 8).unwrap();

        let a = allocator.allocate(big_layout);
        let _b = allocator.allocate(small_layout);
        assert!(!a.is_null());

        allocator.deallocate(a, big_layout);

        let c = allocator.allocate(small_layout);
        assert_eq!(c, a);
    }
}
