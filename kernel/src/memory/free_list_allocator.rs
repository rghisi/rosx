use core::alloc::Layout;
use core::ptr;

struct FreeBlock {
    size: usize,
    next: *mut FreeBlock,
}

const BLOCK_HDR: usize = core::mem::size_of::<FreeBlock>();
const SIZE_SLOT: usize = core::mem::size_of::<usize>();
const BLOCK_ALIGN: usize = core::mem::align_of::<FreeBlock>();

pub struct FreeListAllocator {
    head: *mut FreeBlock,
}

fn align_up(addr: usize, align: usize) -> usize {
    (addr + align - 1) & !(align - 1)
}

impl FreeListAllocator {
    pub fn new(regions: &[(usize, usize)]) -> Self {
        let mut head: *mut FreeBlock = ptr::null_mut();
        for &(base, size) in regions.iter().rev() {
            let aligned = align_up(base, BLOCK_ALIGN);
            let shrink = aligned - base;
            if size <= shrink || size - shrink < BLOCK_HDR {
                continue;
            }
            let block_size = size - shrink;
            unsafe {
                let block = aligned as *mut FreeBlock;
                (*block).size = block_size;
                (*block).next = head;
                head = block;
            }
        }
        FreeListAllocator { head }
    }

    pub unsafe fn allocate(&mut self, layout: Layout) -> *mut u8 {
        if layout.size() == 0 {
            return ptr::null_mut();
        }
        assert!(layout.align() <= BLOCK_ALIGN, "alignment exceeds block alignment");

        let usable = align_up(layout.size(), BLOCK_ALIGN);
        let needed = (SIZE_SLOT + usable).max(BLOCK_HDR);

        let mut prev_next: *mut *mut FreeBlock = &mut self.head;
        let mut current = self.head;
        while !current.is_null() {
            let block_size = (*current).size;
            if block_size >= needed {
                let start = current as usize;
                let remaining = block_size - needed;
                if remaining >= BLOCK_HDR {
                    let split = (start + needed) as *mut FreeBlock;
                    (*split).size = remaining;
                    (*split).next = (*current).next;
                    *prev_next = split;
                    *(start as *mut usize) = needed;
                } else {
                    *prev_next = (*current).next;
                }
                return (start + SIZE_SLOT) as *mut u8;
            }
            prev_next = &mut (*current).next;
            current = (*current).next;
        }
        ptr::null_mut()
    }

    pub unsafe fn deallocate(&mut self, ptr: *mut u8) {
        let start = ptr as usize - SIZE_SLOT;
        let block = start as *mut FreeBlock;

        let mut prev: *mut FreeBlock = ptr::null_mut();
        let mut current = self.head;
        while !current.is_null() && (current as usize) < start {
            prev = current;
            current = (*current).next;
        }

        (*block).next = current;
        if prev.is_null() {
            self.head = block;
        } else {
            (*prev).next = block;
        }

        if !(*block).next.is_null() {
            let block_end = start + (*block).size;
            let next = (*block).next;
            if block_end == next as usize {
                (*block).size += (*next).size;
                (*block).next = (*next).next;
            }
        }

        if !prev.is_null() {
            let prev_end = prev as usize + (*prev).size;
            if prev_end == start {
                (*prev).size += (*block).size;
                (*prev).next = (*block).next;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::alloc::Layout;

    #[test]
    fn allocate_single_block_returns_nonnull_pointer() {
        let mut memory = vec![0u8; 4096];
        let base = memory.as_mut_ptr() as usize;
        let mut alloc = FreeListAllocator::new(&[(base, 4096)]);
        let layout = Layout::from_size_align(64, 8).unwrap();
        let ptr = unsafe { alloc.allocate(layout) };
        assert!(!ptr.is_null());
    }

    #[test]
    fn allocated_pointer_is_within_the_region() {
        let mut memory = vec![0u8; 4096];
        let base = memory.as_mut_ptr() as usize;
        let mut alloc = FreeListAllocator::new(&[(base, 4096)]);
        let layout = Layout::from_size_align(64, 8).unwrap();
        let ptr = unsafe { alloc.allocate(layout) } as usize;
        assert!(ptr >= base);
        assert!(ptr + 64 <= base + 4096);
    }

    #[test]
    fn allocated_pointer_respects_alignment() {
        let mut memory = vec![0u8; 4096];
        let base = memory.as_mut_ptr() as usize;
        let mut alloc = FreeListAllocator::new(&[(base, 4096)]);
        let layout = Layout::from_size_align(64, 8).unwrap();
        let ptr = unsafe { alloc.allocate(layout) } as usize;
        assert_eq!(ptr % 8, 0);
    }

    #[test]
    fn freed_block_can_be_reallocated() {
        let mut memory = vec![0u8; 4096];
        let base = memory.as_mut_ptr() as usize;
        let mut alloc = FreeListAllocator::new(&[(base, 4096)]);
        let layout = Layout::from_size_align(64, 8).unwrap();
        let first = unsafe { alloc.allocate(layout) };
        unsafe { alloc.deallocate(first) };
        let second = unsafe { alloc.allocate(layout) };
        assert_eq!(first, second);
    }

    #[test]
    fn allocate_returns_null_when_out_of_memory() {
        let mut memory = vec![0u8; 128];
        let base = memory.as_mut_ptr() as usize;
        let mut alloc = FreeListAllocator::new(&[(base, 128)]);
        let layout = Layout::from_size_align(256, 8).unwrap();
        let ptr = unsafe { alloc.allocate(layout) };
        assert!(ptr.is_null());
    }

    #[test]
    fn adjacent_freed_blocks_can_serve_a_larger_allocation() {
        // 216 bytes = exactly 3 × 72 (needed per 64-byte allocation), no tail left
        let mut memory = vec![0u8; 216];
        let base = memory.as_mut_ptr() as usize;
        let mut alloc = FreeListAllocator::new(&[(base, 216)]);
        let layout = Layout::from_size_align(64, 8).unwrap();
        let a = unsafe { alloc.allocate(layout) };
        let b = unsafe { alloc.allocate(layout) };
        let _c = unsafe { alloc.allocate(layout) };
        unsafe { alloc.deallocate(a) };
        unsafe { alloc.deallocate(b) };
        // Without coalescing: two 72-byte free blocks, neither fits 88-byte request.
        // With coalescing: merged 144-byte block, fits 88-byte request.
        let large = Layout::from_size_align(80, 8).unwrap();
        let ptr = unsafe { alloc.allocate(large) };
        assert!(!ptr.is_null());
    }

    #[test]
    fn two_allocations_do_not_overlap() {
        let mut memory = vec![0u8; 4096];
        let base = memory.as_mut_ptr() as usize;
        let mut alloc = FreeListAllocator::new(&[(base, 4096)]);
        let layout = Layout::from_size_align(64, 8).unwrap();
        let first = unsafe { alloc.allocate(layout) } as usize;
        let second = unsafe { alloc.allocate(layout) } as usize;
        let no_overlap = second >= first + 64 || first >= second + 64;
        assert!(no_overlap);
    }

    #[test]
    fn allocates_from_second_region_when_first_is_exhausted() {
        // Region 1 is 80 bytes — fits at most one 64-byte allocation (needed=72, <=80, tail<BLOCK_HDR).
        // Region 2 is 4096 bytes. After region 1 is full, the next alloc must come from region 2.
        let mut mem1 = vec![0u8; 80];
        let mut mem2 = vec![0u8; 4096];
        let base1 = mem1.as_mut_ptr() as usize;
        let base2 = mem2.as_mut_ptr() as usize;
        let mut alloc = FreeListAllocator::new(&[(base1, 80), (base2, 4096)]);
        let layout = Layout::from_size_align(64, 8).unwrap();
        let first = unsafe { alloc.allocate(layout) };
        assert!(!first.is_null());
        let second = unsafe { alloc.allocate(layout) };
        assert!(!second.is_null());
        let second_addr = second as usize;
        assert!(second_addr >= base2 && second_addr + 64 <= base2 + 4096);
    }
}
