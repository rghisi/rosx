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
}
