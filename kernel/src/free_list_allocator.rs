use core::alloc::Layout;

use crate::bitmap_chunk_allocator::BitmapChunkAllocator;

const FREE_BLOCK_SIZE: usize = core::mem::size_of::<FreeBlock>();
const FREE_BLOCK_ALIGN: usize = core::mem::align_of::<FreeBlock>();

struct FreeBlock {
    size: usize,
    next: *mut FreeBlock,
}

struct ChunkRecord {
    size: usize,
    next: *mut ChunkRecord,
    prev: *mut ChunkRecord,
}

struct ChunkRecordList {
    head: *mut ChunkRecord,
}

impl ChunkRecordList {
    fn new() -> Self {
        ChunkRecordList {
            head: core::ptr::null_mut(),
        }
    }

    fn insert(&mut self, ptr: *mut u8, size: usize) {
        // Safety: ptr points to a writable memory region of at least
        // size_of::<ChunkRecord>() bytes, provided by the chunk allocator.
        unsafe {
            let record = ptr as *mut ChunkRecord;
            (*record).size = size;
            (*record).prev = core::ptr::null_mut();
            (*record).next = self.head;
            if !self.head.is_null() {
                (*self.head).prev = record;
            }
            self.head = record;
        }
    }

    fn find(&self, address: usize, size: usize) -> *mut ChunkRecord {
        // Safety: we only follow pointers set by insert(), which writes
        // valid ChunkRecords into chunk memory.
        unsafe {
            let mut current = self.head;
            while !current.is_null() {
                if current as usize == address && (*current).size == size {
                    return current;
                }
                current = (*current).next;
            }
        }
        core::ptr::null_mut()
    }

    fn remove(&mut self, record: *mut ChunkRecord) {
        // Safety: record is a valid pointer previously returned by insert()
        // or find(). We unlink it by patching its neighbors' pointers.
        unsafe {
            let prev = (*record).prev;
            let next = (*record).next;
            if !prev.is_null() {
                (*prev).next = next;
            } else {
                self.head = next;
            }
            if !next.is_null() {
                (*next).prev = prev;
            }
        }
    }
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
        let mut new_start = ptr as usize;
        let mut new_size = size;

        // Safety: we traverse free list pointers that were set during chunk
        // acquisition or previous deallocations. We remove at most two
        // neighbors (left and right) and merge their ranges into the new block.
        unsafe {
            let mut prev: *mut FreeBlock = core::ptr::null_mut();
            let mut current = self.free_list;

            while !current.is_null() {
                let block_start = current as usize;
                let block_end = block_start + (*current).size;
                let new_end = new_start + new_size;
                let next = (*current).next;

                if block_end == new_start {
                    new_start = block_start;
                    new_size += (*current).size;
                    self.remove_block(prev, current);
                    current = if prev.is_null() { self.free_list } else { (*prev).next };
                } else if new_end == block_start {
                    new_size += (*current).size;
                    self.remove_block(prev, current);
                    current = if prev.is_null() { self.free_list } else { (*prev).next };
                } else {
                    prev = current;
                    current = next;
                }
            }

            let block = new_start as *mut FreeBlock;
            (*block).size = new_size;
            (*block).next = self.free_list;
            self.free_list = block;
        }
    }

    unsafe fn remove_block(&mut self, prev: *mut FreeBlock, current: *mut FreeBlock) {
        unsafe {
            let next = (*current).next;
            if prev.is_null() {
                self.free_list = next;
            } else {
                (*prev).next = next;
            }
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
    fn coalesce_two_adjacent_blocks() {
        let mut memory = vec![0u8; 10 * CHUNK_SIZE];
        let base = memory.as_mut_ptr() as usize;
        let chunk_allocator = BitmapChunkAllocator::with_chunk_size(
            CHUNK_SIZE,
            &[(base, memory.len())],
        );

        let mut allocator = FreeListAllocator::new(chunk_allocator);

        let quarter = Layout::from_size_align(CHUNK_SIZE / 4, 8).unwrap();
        let mut all = Vec::new();
        loop {
            let ptr = allocator.allocate(quarter);
            if ptr.is_null() {
                break;
            }
            all.push(ptr);
        }
        assert!(all.len() >= 4);

        let a = all[0];
        let b = all[1];
        assert_eq!(b as usize - a as usize, CHUNK_SIZE / 4);

        allocator.deallocate(a, quarter);
        allocator.deallocate(b, quarter);

        let half = Layout::from_size_align(CHUNK_SIZE / 2, 8).unwrap();
        let ptr = allocator.allocate(half);
        assert!(
            !ptr.is_null(),
            "coalescing two adjacent blocks should yield a block large enough for combined size"
        );
    }

    #[test]
    fn coalesce_three_adjacent_blocks() {
        let mut memory = vec![0u8; 10 * CHUNK_SIZE];
        let base = memory.as_mut_ptr() as usize;
        let chunk_allocator = BitmapChunkAllocator::with_chunk_size(
            CHUNK_SIZE,
            &[(base, memory.len())],
        );

        let mut allocator = FreeListAllocator::new(chunk_allocator);

        let quarter = Layout::from_size_align(CHUNK_SIZE / 4, 8).unwrap();
        let mut all = Vec::new();
        loop {
            let ptr = allocator.allocate(quarter);
            if ptr.is_null() {
                break;
            }
            all.push(ptr);
        }
        assert!(all.len() >= 4);

        let a = all[0];
        let b = all[1];
        let c = all[2];
        assert_eq!(b as usize - a as usize, CHUNK_SIZE / 4);
        assert_eq!(c as usize - b as usize, CHUNK_SIZE / 4);

        allocator.deallocate(a, quarter);
        allocator.deallocate(c, quarter);
        allocator.deallocate(b, quarter);

        let three_quarters = Layout::from_size_align(3 * CHUNK_SIZE / 4, 8).unwrap();
        let ptr = allocator.allocate(three_quarters);
        assert!(
            !ptr.is_null(),
            "coalescing three adjacent blocks should yield a block large enough for combined size"
        );
    }

    #[test]
    fn no_coalesce_when_gap_exists() {
        let mut memory = vec![0u8; 10 * CHUNK_SIZE];
        let base = memory.as_mut_ptr() as usize;
        let chunk_allocator = BitmapChunkAllocator::with_chunk_size(
            CHUNK_SIZE,
            &[(base, memory.len())],
        );

        let mut allocator = FreeListAllocator::new(chunk_allocator);

        let quarter = Layout::from_size_align(CHUNK_SIZE / 4, 8).unwrap();
        let mut all = Vec::new();
        loop {
            let ptr = allocator.allocate(quarter);
            if ptr.is_null() {
                break;
            }
            all.push(ptr);
        }
        assert!(all.len() >= 4);

        allocator.deallocate(all[0], quarter);
        allocator.deallocate(all[2], quarter);

        let half = Layout::from_size_align(CHUNK_SIZE / 2, 8).unwrap();
        let ptr = allocator.allocate(half);
        assert!(
            ptr.is_null(),
            "non-adjacent freed blocks should not be coalesced"
        );
    }

    #[test]
    fn coalesce_preserves_alignment() {
        let mut memory = vec![0u8; 10 * CHUNK_SIZE];
        let base = memory.as_mut_ptr() as usize;
        let chunk_allocator = BitmapChunkAllocator::with_chunk_size(
            CHUNK_SIZE,
            &[(base, memory.len())],
        );

        let mut allocator = FreeListAllocator::new(chunk_allocator);

        let quarter = Layout::from_size_align(CHUNK_SIZE / 4, 8).unwrap();
        let mut all = Vec::new();
        loop {
            let ptr = allocator.allocate(quarter);
            if ptr.is_null() {
                break;
            }
            all.push(ptr);
        }
        assert!(all.len() >= 4);

        let a = all[0];
        let b = all[1];
        assert_eq!(b as usize - a as usize, CHUNK_SIZE / 4);

        allocator.deallocate(a, quarter);
        allocator.deallocate(b, quarter);

        let aligned_layout = Layout::from_size_align(CHUNK_SIZE / 4 + 1, 64).unwrap();
        let ptr = allocator.allocate(aligned_layout);
        assert!(
            !ptr.is_null(),
            "coalesced block should satisfy aligned allocation larger than a single block"
        );
        assert_eq!(
            (ptr as usize) % 64,
            0,
            "pointer should be aligned to requested alignment"
        );
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

    #[test]
    fn deallocate_full_chunk_returns_to_bitmap() {
        let mut memory = vec![0u8; 10 * CHUNK_SIZE];
        let base = memory.as_mut_ptr() as usize;
        let chunk_allocator = BitmapChunkAllocator::with_chunk_size(
            CHUNK_SIZE,
            &[(base, memory.len())],
        );

        let mut allocator = FreeListAllocator::new(chunk_allocator);

        let layout = Layout::from_size_align(CHUNK_SIZE, 8).unwrap();
        let ptr = allocator.allocate(layout);
        assert!(!ptr.is_null());
        assert_eq!(allocator.chunk_allocator.used_chunks(), 1);

        allocator.deallocate(ptr, layout);
        assert_eq!(
            allocator.chunk_allocator.used_chunks(),
            0,
            "freeing a full chunk should return it to the bitmap allocator"
        );
    }

    #[test]
    fn deallocate_coalesced_chunk_returns_to_bitmap() {
        let mut memory = vec![0u8; 10 * CHUNK_SIZE];
        let base = memory.as_mut_ptr() as usize;
        let chunk_allocator = BitmapChunkAllocator::with_chunk_size(
            CHUNK_SIZE,
            &[(base, memory.len())],
        );

        let mut allocator = FreeListAllocator::new(chunk_allocator);

        let half = Layout::from_size_align(CHUNK_SIZE / 2, 8).unwrap();
        let a = allocator.allocate(half);
        let b = allocator.allocate(half);
        assert!(!a.is_null());
        assert!(!b.is_null());
        assert_eq!(allocator.chunk_allocator.used_chunks(), 1);

        allocator.deallocate(a, half);
        assert_eq!(
            allocator.chunk_allocator.used_chunks(),
            1,
            "partial free should not return chunk to bitmap"
        );

        allocator.deallocate(b, half);
        assert_eq!(
            allocator.chunk_allocator.used_chunks(),
            0,
            "coalesced full chunk should be returned to bitmap allocator"
        );
    }

    #[test]
    fn deallocate_partial_chunk_not_returned_to_bitmap() {
        let mut memory = vec![0u8; 10 * CHUNK_SIZE];
        let base = memory.as_mut_ptr() as usize;
        let chunk_allocator = BitmapChunkAllocator::with_chunk_size(
            CHUNK_SIZE,
            &[(base, memory.len())],
        );

        let mut allocator = FreeListAllocator::new(chunk_allocator);

        let quarter = Layout::from_size_align(CHUNK_SIZE / 4, 8).unwrap();
        let a = allocator.allocate(quarter);
        let _b = allocator.allocate(quarter);
        let c = allocator.allocate(quarter);
        let _d = allocator.allocate(quarter);
        assert!(!a.is_null());
        assert!(!c.is_null());
        assert_eq!(allocator.chunk_allocator.used_chunks(), 1);

        allocator.deallocate(a, quarter);
        allocator.deallocate(c, quarter);
        assert_eq!(
            allocator.chunk_allocator.used_chunks(),
            1,
            "partially freed chunk must not be returned to bitmap"
        );
    }

    #[test]
    fn deallocate_multi_chunk_allocation_returns_to_bitmap() {
        let mut memory = vec![0u8; 10 * CHUNK_SIZE];
        let base = memory.as_mut_ptr() as usize;
        let chunk_allocator = BitmapChunkAllocator::with_chunk_size(
            CHUNK_SIZE,
            &[(base, memory.len())],
        );

        let mut allocator = FreeListAllocator::new(chunk_allocator);

        let two_chunks = Layout::from_size_align(2 * CHUNK_SIZE, 8).unwrap();
        let ptr = allocator.allocate(two_chunks);
        assert!(!ptr.is_null());
        assert_eq!(allocator.chunk_allocator.used_chunks(), 2);

        allocator.deallocate(ptr, two_chunks);
        assert_eq!(
            allocator.chunk_allocator.used_chunks(),
            0,
            "freeing a multi-chunk allocation should return all chunks to bitmap"
        );
    }

    #[test]
    fn chunk_record_list_starts_empty() {
        let list = ChunkRecordList::new();
        assert!(list.head.is_null());
    }

    #[test]
    fn insert_single_chunk_record() {
        let mut buffer = vec![0usize; 512];
        let ptr = buffer.as_mut_ptr() as *mut u8;

        let mut list = ChunkRecordList::new();
        list.insert(ptr, 4096);

        assert!(!list.head.is_null());
        unsafe {
            assert_eq!((*list.head).size, 4096);
            assert!((*list.head).next.is_null());
            assert!((*list.head).prev.is_null());
        }
    }

    #[test]
    fn insert_two_chunk_records() {
        let mut buffer1 = vec![0usize; 512];
        let mut buffer2 = vec![0usize; 512];
        let ptr1 = buffer1.as_mut_ptr() as *mut u8;
        let ptr2 = buffer2.as_mut_ptr() as *mut u8;

        let mut list = ChunkRecordList::new();
        list.insert(ptr1, 4096);
        list.insert(ptr2, 4096);

        unsafe {
            assert_eq!(list.head, ptr2 as *mut ChunkRecord);
            assert_eq!((*list.head).next, ptr1 as *mut ChunkRecord);
            assert!((*list.head).prev.is_null());

            let second = (*list.head).next;
            assert_eq!((*second).prev, list.head);
            assert!((*second).next.is_null());
        }
    }

    #[test]
    fn find_chunk_record_by_address_and_size() {
        let mut buffer = vec![0usize; 512];
        let ptr = buffer.as_mut_ptr() as *mut u8;

        let mut list = ChunkRecordList::new();
        list.insert(ptr, 4096);

        let found = list.find(ptr as usize, 4096);
        assert!(!found.is_null());
        assert_eq!(found, ptr as *mut ChunkRecord);
    }

    #[test]
    fn find_returns_null_when_address_not_found() {
        let mut buffer = vec![0usize; 512];
        let ptr = buffer.as_mut_ptr() as *mut u8;

        let mut list = ChunkRecordList::new();
        list.insert(ptr, 4096);

        let found = list.find(0xDEAD_BEEF, 4096);
        assert!(found.is_null());
    }

    #[test]
    fn find_returns_null_when_size_does_not_match() {
        let mut buffer = vec![0usize; 512];
        let ptr = buffer.as_mut_ptr() as *mut u8;

        let mut list = ChunkRecordList::new();
        list.insert(ptr, 4096);

        let found = list.find(ptr as usize, 8192);
        assert!(found.is_null());
    }

    #[test]
    fn remove_head_record() {
        let mut buf1 = vec![0usize; 512];
        let mut buf2 = vec![0usize; 512];
        let ptr1 = buf1.as_mut_ptr() as *mut u8;
        let ptr2 = buf2.as_mut_ptr() as *mut u8;

        let mut list = ChunkRecordList::new();
        list.insert(ptr1, 4096);
        list.insert(ptr2, 4096);

        let head = list.head;
        list.remove(head);

        assert_eq!(list.head, ptr1 as *mut ChunkRecord);
        unsafe {
            assert!((*list.head).prev.is_null());
            assert!((*list.head).next.is_null());
        }
    }

    #[test]
    fn remove_tail_record() {
        let mut buf1 = vec![0usize; 512];
        let mut buf2 = vec![0usize; 512];
        let ptr1 = buf1.as_mut_ptr() as *mut u8;
        let ptr2 = buf2.as_mut_ptr() as *mut u8;

        let mut list = ChunkRecordList::new();
        list.insert(ptr1, 4096);
        list.insert(ptr2, 4096);

        let tail = ptr1 as *mut ChunkRecord;
        list.remove(tail);

        assert_eq!(list.head, ptr2 as *mut ChunkRecord);
        unsafe {
            assert!((*list.head).next.is_null());
        }
    }

    #[test]
    fn remove_middle_record() {
        let mut buf1 = vec![0usize; 512];
        let mut buf2 = vec![0usize; 512];
        let mut buf3 = vec![0usize; 512];
        let ptr1 = buf1.as_mut_ptr() as *mut u8;
        let ptr2 = buf2.as_mut_ptr() as *mut u8;
        let ptr3 = buf3.as_mut_ptr() as *mut u8;

        let mut list = ChunkRecordList::new();
        list.insert(ptr1, 4096);
        list.insert(ptr2, 4096);
        list.insert(ptr3, 4096);

        let middle = ptr2 as *mut ChunkRecord;
        list.remove(middle);

        unsafe {
            assert_eq!(list.head, ptr3 as *mut ChunkRecord);
            assert_eq!((*list.head).next, ptr1 as *mut ChunkRecord);

            let tail = (*list.head).next;
            assert_eq!((*tail).prev, list.head);
            assert!((*tail).next.is_null());
        }
    }

    #[test]
    fn remove_only_record_leaves_empty_list() {
        let mut buffer = vec![0usize; 512];
        let ptr = buffer.as_mut_ptr() as *mut u8;

        let mut list = ChunkRecordList::new();
        list.insert(ptr, 4096);
        list.remove(list.head);

        assert!(list.head.is_null());
    }
}
