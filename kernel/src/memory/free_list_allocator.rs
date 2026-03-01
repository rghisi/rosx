use core::alloc::Layout;
use core::ptr;

struct FreeBlock {
    size: usize,
    next: *mut FreeBlock,
}

struct AllocHeader {
    size: usize,
    owner: BlockOwner,
    next: *mut AllocHeader,
}

#[derive(Copy, Clone, PartialEq, Eq)]
pub(crate) enum BlockOwner {
    Kernel,
    Task(usize),
}

const BLOCK_HDR: usize = core::mem::size_of::<FreeBlock>();
const BLOCK_ALIGN: usize = core::mem::align_of::<FreeBlock>();
const ALLOC_HDR: usize = core::mem::size_of::<AllocHeader>();

pub struct FreeListAllocator {
    head: *mut FreeBlock,
    alloc_head: *mut AllocHeader,
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
        FreeListAllocator { head, alloc_head: ptr::null_mut() }
    }

    pub unsafe fn allocate(&mut self, layout: Layout, owner: BlockOwner) -> *mut u8 {
        if layout.size() == 0 {
            return ptr::null_mut();
        }
        assert!(layout.align() <= BLOCK_ALIGN, "alignment exceeds block alignment");

        let usable = align_up(layout.size(), BLOCK_ALIGN);
        let needed = (ALLOC_HDR + usable).max(BLOCK_HDR);

        let mut prev_next: *mut *mut FreeBlock = &mut self.head;
        let mut current = self.head;
        while !current.is_null() {
            let block_size = (*current).size;
            if block_size >= needed {
                let start = current as usize;
                let remaining = block_size - needed;
                let used_size;
                if remaining >= BLOCK_HDR {
                    let split = (start + needed) as *mut FreeBlock;
                    (*split).size = remaining;
                    (*split).next = (*current).next;
                    *prev_next = split;
                    used_size = needed;
                } else {
                    *prev_next = (*current).next;
                    used_size = block_size;
                }
                let header = start as *mut AllocHeader;
                (*header).size = used_size;
                (*header).owner = owner;
                (*header).next = self.alloc_head;
                self.alloc_head = header;
                return (start + ALLOC_HDR) as *mut u8;
            }
            prev_next = &mut (*current).next;
            current = (*current).next;
        }
        ptr::null_mut()
    }

    pub unsafe fn deallocate(&mut self, ptr: *mut u8) {
        let start = ptr as usize - ALLOC_HDR;
        let header = start as *mut AllocHeader;
        let block_size = (*header).size;

        let mut prev: *mut AllocHeader = ptr::null_mut();
        let mut current = self.alloc_head;
        while !current.is_null() && current != header {
            prev = current;
            current = (*current).next;
        }
        if prev.is_null() {
            self.alloc_head = (*header).next;
        } else {
            (*prev).next = (*header).next;
        }

        self.insert_free_block(start, block_size);
    }

    pub unsafe fn deallocate_by_owner(&mut self, task_id: usize) {
        let target = BlockOwner::Task(task_id);
        let mut prev_next: *mut *mut AllocHeader = &mut self.alloc_head;
        let mut current = self.alloc_head;
        while !current.is_null() {
            let next = (*current).next;
            if (*current).owner == target {
                *prev_next = next;
                let start = current as usize;
                let block_size = (*current).size;
                self.insert_free_block(start, block_size);
            } else {
                prev_next = &mut (*current).next;
            }
            current = next;
        }
    }

    unsafe fn insert_free_block(&mut self, start: usize, block_size: usize) {
        let block = start as *mut FreeBlock;
        (*block).size = block_size;

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

    fn needed_for(size: usize) -> usize {
        let usable = align_up(size, BLOCK_ALIGN);
        (ALLOC_HDR + usable).max(BLOCK_HDR)
    }

    #[test]
    fn allocate_single_block_returns_nonnull_pointer() {
        let mut memory = vec![0u8; 4096];
        let base = memory.as_mut_ptr() as usize;
        let mut alloc = FreeListAllocator::new(&[(base, 4096)]);
        let layout = Layout::from_size_align(64, 8).unwrap();
        let ptr = unsafe { alloc.allocate(layout, BlockOwner::Kernel) };
        assert!(!ptr.is_null());
    }

    #[test]
    fn allocated_pointer_is_within_the_region() {
        let mut memory = vec![0u8; 4096];
        let base = memory.as_mut_ptr() as usize;
        let mut alloc = FreeListAllocator::new(&[(base, 4096)]);
        let layout = Layout::from_size_align(64, 8).unwrap();
        let ptr = unsafe { alloc.allocate(layout, BlockOwner::Kernel) } as usize;
        assert!(ptr >= base);
        assert!(ptr + 64 <= base + 4096);
    }

    #[test]
    fn allocated_pointer_respects_alignment() {
        let mut memory = vec![0u8; 4096];
        let base = memory.as_mut_ptr() as usize;
        let mut alloc = FreeListAllocator::new(&[(base, 4096)]);
        let layout = Layout::from_size_align(64, 8).unwrap();
        let ptr = unsafe { alloc.allocate(layout, BlockOwner::Kernel) } as usize;
        assert_eq!(ptr % 8, 0);
    }

    #[test]
    fn freed_block_can_be_reallocated() {
        let mut memory = vec![0u8; 4096];
        let base = memory.as_mut_ptr() as usize;
        let mut alloc = FreeListAllocator::new(&[(base, 4096)]);
        let layout = Layout::from_size_align(64, 8).unwrap();
        let first = unsafe { alloc.allocate(layout, BlockOwner::Kernel) };
        unsafe { alloc.deallocate(first) };
        let second = unsafe { alloc.allocate(layout, BlockOwner::Kernel) };
        assert_eq!(first, second);
    }

    #[test]
    fn allocate_returns_null_when_out_of_memory() {
        let mut memory = vec![0u8; 128];
        let base = memory.as_mut_ptr() as usize;
        let mut alloc = FreeListAllocator::new(&[(base, 128)]);
        let layout = Layout::from_size_align(256, 8).unwrap();
        let ptr = unsafe { alloc.allocate(layout, BlockOwner::Kernel) };
        assert!(ptr.is_null());
    }

    #[test]
    fn adjacent_freed_blocks_can_serve_a_larger_allocation() {
        // exactly 3 allocations of 64 bytes, no tail remaining
        let region_size = 3 * needed_for(64);
        let mut memory = vec![0u8; region_size];
        let base = memory.as_mut_ptr() as usize;
        let mut alloc = FreeListAllocator::new(&[(base, region_size)]);
        let layout = Layout::from_size_align(64, 8).unwrap();
        let a = unsafe { alloc.allocate(layout, BlockOwner::Kernel) };
        let b = unsafe { alloc.allocate(layout, BlockOwner::Kernel) };
        let _c = unsafe { alloc.allocate(layout, BlockOwner::Kernel) };
        unsafe { alloc.deallocate(a) };
        unsafe { alloc.deallocate(b) };
        // without coalescing: two needed_for(64)-byte blocks, neither fits needed_for(80)
        // with coalescing: merged 2*needed_for(64) block fits
        let large = Layout::from_size_align(80, 8).unwrap();
        let ptr = unsafe { alloc.allocate(large, BlockOwner::Kernel) };
        assert!(!ptr.is_null());
    }

    #[test]
    fn two_allocations_do_not_overlap() {
        let mut memory = vec![0u8; 4096];
        let base = memory.as_mut_ptr() as usize;
        let mut alloc = FreeListAllocator::new(&[(base, 4096)]);
        let layout = Layout::from_size_align(64, 8).unwrap();
        let first = unsafe { alloc.allocate(layout, BlockOwner::Kernel) } as usize;
        let second = unsafe { alloc.allocate(layout, BlockOwner::Kernel) } as usize;
        let no_overlap = second >= first + 64 || first >= second + 64;
        assert!(no_overlap);
    }

    #[test]
    fn allocates_from_second_region_when_first_is_exhausted() {
        // Region 1 fits exactly one 64-byte allocation (needed_for(64) bytes, no split tail).
        // Region 2 is large. After region 1 is full the next alloc must come from region 2.
        let region1_size = needed_for(64) + BLOCK_ALIGN;
        let mut mem1 = vec![0u8; region1_size];
        let mut mem2 = vec![0u8; 4096];
        let base1 = mem1.as_mut_ptr() as usize;
        let base2 = mem2.as_mut_ptr() as usize;
        let mut alloc = FreeListAllocator::new(&[(base1, region1_size), (base2, 4096)]);
        let layout = Layout::from_size_align(64, 8).unwrap();
        let first = unsafe { alloc.allocate(layout, BlockOwner::Kernel) };
        assert!(!first.is_null());
        let second = unsafe { alloc.allocate(layout, BlockOwner::Kernel) };
        assert!(!second.is_null());
        let second_addr = second as usize;
        assert!(second_addr >= base2 && second_addr + 64 <= base2 + 4096);
    }

    #[test]
    fn allocate_with_kernel_owner_returns_non_null() {
        let mut memory = vec![0u8; 4096];
        let base = memory.as_mut_ptr() as usize;
        let mut alloc = FreeListAllocator::new(&[(base, 4096)]);
        let layout = Layout::from_size_align(64, 8).unwrap();
        let ptr = unsafe { alloc.allocate(layout, BlockOwner::Kernel) };
        assert!(!ptr.is_null());
    }

    #[test]
    fn allocate_with_task_owner_returns_non_null() {
        let mut memory = vec![0u8; 4096];
        let base = memory.as_mut_ptr() as usize;
        let mut alloc = FreeListAllocator::new(&[(base, 4096)]);
        let layout = Layout::from_size_align(64, 8).unwrap();
        let ptr = unsafe { alloc.allocate(layout, BlockOwner::Task(1)) };
        assert!(!ptr.is_null());
    }

    #[test]
    fn deallocate_by_owner_frees_task_blocks() {
        // Two allocations exactly fill the region; a third would fail.
        // After freeing the task block, a new allocation must succeed.
        let region_size = 2 * needed_for(64);
        let mut memory = vec![0u8; region_size];
        let base = memory.as_mut_ptr() as usize;
        let mut alloc = FreeListAllocator::new(&[(base, region_size)]);
        let layout = Layout::from_size_align(64, 8).unwrap();
        unsafe { alloc.allocate(layout, BlockOwner::Task(42)) };
        unsafe { alloc.allocate(layout, BlockOwner::Kernel) };
        assert!(unsafe { alloc.allocate(layout, BlockOwner::Kernel) }.is_null());
        unsafe { alloc.deallocate_by_owner(42) };
        assert!(!unsafe { alloc.allocate(layout, BlockOwner::Kernel) }.is_null());
    }

    #[test]
    fn deallocate_by_owner_does_not_free_kernel_blocks() {
        let region_size = 2 * needed_for(64);
        let mut memory = vec![0u8; region_size];
        let base = memory.as_mut_ptr() as usize;
        let mut alloc = FreeListAllocator::new(&[(base, region_size)]);
        let layout = Layout::from_size_align(64, 8).unwrap();
        unsafe { alloc.allocate(layout, BlockOwner::Kernel) };
        unsafe { alloc.allocate(layout, BlockOwner::Kernel) };
        unsafe { alloc.deallocate_by_owner(99) };
        assert!(unsafe { alloc.allocate(layout, BlockOwner::Kernel) }.is_null());
    }

    #[test]
    fn deallocate_by_owner_only_frees_matching_task() {
        // task-1 owns slots 0 and 2, task-2 owns slot 1 (sits between them, so no coalescing).
        // After freeing task-1, exactly two 64-byte slots become available, not three.
        let region_size = 3 * needed_for(64);
        let mut memory = vec![0u8; region_size];
        let base = memory.as_mut_ptr() as usize;
        let mut alloc = FreeListAllocator::new(&[(base, region_size)]);
        let layout = Layout::from_size_align(64, 8).unwrap();
        unsafe { alloc.allocate(layout, BlockOwner::Task(1)) };
        unsafe { alloc.allocate(layout, BlockOwner::Task(2)) };
        unsafe { alloc.allocate(layout, BlockOwner::Task(1)) };
        unsafe { alloc.deallocate_by_owner(1) };
        assert!(!unsafe { alloc.allocate(layout, BlockOwner::Kernel) }.is_null());
        assert!(!unsafe { alloc.allocate(layout, BlockOwner::Kernel) }.is_null());
        assert!(unsafe { alloc.allocate(layout, BlockOwner::Kernel) }.is_null());
    }
}
