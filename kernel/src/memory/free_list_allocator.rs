use core::alloc::Layout;
use core::ptr;

use crate::memory::MemoryBlocks;

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

#[derive(Debug, PartialEq)]
pub(crate) enum AllocError {
    OutOfMemory,
    AlignmentUnsupported,
}

pub struct FreeListAllocator {
    head: *mut FreeBlock,
    alloc_head: *mut AllocHeader,
}

fn align_up(addr: usize, align: usize) -> usize {
    (addr + align - 1) & !(align - 1)
}

impl FreeListAllocator {
    pub fn new(memory_blocks: &MemoryBlocks) -> Self {
        let mut head: *mut FreeBlock = ptr::null_mut();
        for block in memory_blocks.blocks[..memory_blocks.count].iter().rev() {
            let aligned = align_up(block.start, BLOCK_ALIGN);
            let shrink = aligned - block.start;
            if block.size <= shrink || block.size - shrink < BLOCK_HDR {
                continue;
            }
            let block_size = block.size - shrink;
            unsafe {
                let fb = aligned as *mut FreeBlock;
                (*fb).size = block_size;
                (*fb).next = head;
                head = fb;
            }
        }
        FreeListAllocator { head, alloc_head: ptr::null_mut() }
    }

    pub unsafe fn allocate(&mut self, layout: Layout, owner: BlockOwner) -> Result<*mut u8, AllocError> {
        if layout.size() == 0 {
            return Err(AllocError::OutOfMemory);
        }
        if layout.align() > BLOCK_ALIGN {
            return Err(AllocError::AlignmentUnsupported);
        }

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
                return Ok((start + ALLOC_HDR) as *mut u8);
            }
            prev_next = &mut (*current).next;
            current = (*current).next;
        }
        Err(AllocError::OutOfMemory)
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
    use crate::memory::{MemoryBlock, MemoryBlocks, MAX_MEMORY_BLOCKS};

    fn make_allocator(regions: &[(usize, usize)]) -> FreeListAllocator {
        let mut blocks = [MemoryBlock { start: 0, size: 0 }; MAX_MEMORY_BLOCKS];
        for (i, &(start, size)) in regions.iter().enumerate() {
            blocks[i] = MemoryBlock { start, size };
        }
        FreeListAllocator::new(&MemoryBlocks { blocks, count: regions.len() })
    }

    fn needed_for(size: usize) -> usize {
        let usable = align_up(size, BLOCK_ALIGN);
        (ALLOC_HDR + usable).max(BLOCK_HDR)
    }

    #[test]
    fn allocate_single_block_returns_ok() {
        let mut memory = vec![0u8; 4096];
        let base = memory.as_mut_ptr() as usize;
        let mut alloc = make_allocator(&[(base, 4096)]);
        let layout = Layout::from_size_align(64, 8).unwrap();
        let result = unsafe { alloc.allocate(layout, BlockOwner::Kernel) };
        assert!(result.is_ok());
    }

    #[test]
    fn allocated_pointer_is_within_the_region() {
        let mut memory = vec![0u8; 4096];
        let base = memory.as_mut_ptr() as usize;
        let mut alloc = make_allocator(&[(base, 4096)]);
        let layout = Layout::from_size_align(64, 8).unwrap();
        let ptr = unsafe { alloc.allocate(layout, BlockOwner::Kernel) }.unwrap() as usize;
        assert!(ptr >= base);
        assert!(ptr + 64 <= base + 4096);
    }

    #[test]
    fn allocated_pointer_respects_alignment() {
        let mut memory = vec![0u8; 4096];
        let base = memory.as_mut_ptr() as usize;
        let mut alloc = make_allocator(&[(base, 4096)]);
        let layout = Layout::from_size_align(64, 8).unwrap();
        let ptr = unsafe { alloc.allocate(layout, BlockOwner::Kernel) }.unwrap() as usize;
        assert_eq!(ptr % 8, 0);
    }

    #[test]
    fn freed_block_can_be_reallocated() {
        let mut memory = vec![0u8; 4096];
        let base = memory.as_mut_ptr() as usize;
        let mut alloc = make_allocator(&[(base, 4096)]);
        let layout = Layout::from_size_align(64, 8).unwrap();
        let first = unsafe { alloc.allocate(layout, BlockOwner::Kernel) }.unwrap();
        unsafe { alloc.deallocate(first) };
        let second = unsafe { alloc.allocate(layout, BlockOwner::Kernel) }.unwrap();
        assert_eq!(first, second);
    }

    #[test]
    fn allocate_returns_out_of_memory_when_exhausted() {
        let mut memory = vec![0u8; 128];
        let base = memory.as_mut_ptr() as usize;
        let mut alloc = make_allocator(&[(base, 128)]);
        let layout = Layout::from_size_align(256, 8).unwrap();
        let result = unsafe { alloc.allocate(layout, BlockOwner::Kernel) };
        assert!(matches!(result, Err(AllocError::OutOfMemory)));
    }

    #[test]
    fn allocate_returns_alignment_unsupported_when_alignment_exceeds_block_align() {
        let mut memory = vec![0u8; 4096];
        let base = memory.as_mut_ptr() as usize;
        let mut alloc = make_allocator(&[(base, 4096)]);
        let layout = Layout::from_size_align(64, BLOCK_ALIGN * 2).unwrap();
        let result = unsafe { alloc.allocate(layout, BlockOwner::Kernel) };
        assert!(matches!(result, Err(AllocError::AlignmentUnsupported)));
    }

    #[test]
    fn adjacent_freed_blocks_can_serve_a_larger_allocation() {
        let region_size = 3 * needed_for(64);
        let mut memory = vec![0u8; region_size];
        let base = memory.as_mut_ptr() as usize;
        let mut alloc = make_allocator(&[(base, region_size)]);
        let layout = Layout::from_size_align(64, 8).unwrap();
        let a = unsafe { alloc.allocate(layout, BlockOwner::Kernel) }.unwrap();
        let b = unsafe { alloc.allocate(layout, BlockOwner::Kernel) }.unwrap();
        let _c = unsafe { alloc.allocate(layout, BlockOwner::Kernel) }.unwrap();
        unsafe { alloc.deallocate(a) };
        unsafe { alloc.deallocate(b) };
        let large = Layout::from_size_align(80, 8).unwrap();
        let result = unsafe { alloc.allocate(large, BlockOwner::Kernel) };
        assert!(result.is_ok());
    }

    #[test]
    fn two_allocations_do_not_overlap() {
        let mut memory = vec![0u8; 4096];
        let base = memory.as_mut_ptr() as usize;
        let mut alloc = make_allocator(&[(base, 4096)]);
        let layout = Layout::from_size_align(64, 8).unwrap();
        let first = unsafe { alloc.allocate(layout, BlockOwner::Kernel) }.unwrap() as usize;
        let second = unsafe { alloc.allocate(layout, BlockOwner::Kernel) }.unwrap() as usize;
        let no_overlap = second >= first + 64 || first >= second + 64;
        assert!(no_overlap);
    }

    #[test]
    fn allocates_from_second_region_when_first_is_exhausted() {
        let region1_size = needed_for(64) + BLOCK_ALIGN;
        let mut mem1 = vec![0u8; region1_size];
        let mut mem2 = vec![0u8; 4096];
        let base1 = mem1.as_mut_ptr() as usize;
        let base2 = mem2.as_mut_ptr() as usize;
        let mut alloc = make_allocator(&[(base1, region1_size), (base2, 4096)]);
        let layout = Layout::from_size_align(64, 8).unwrap();
        unsafe { alloc.allocate(layout, BlockOwner::Kernel) }.unwrap();
        let second = unsafe { alloc.allocate(layout, BlockOwner::Kernel) }.unwrap();
        let second_addr = second as usize;
        assert!(second_addr >= base2 && second_addr + 64 <= base2 + 4096);
    }

    #[test]
    fn allocate_with_kernel_owner_returns_ok() {
        let mut memory = vec![0u8; 4096];
        let base = memory.as_mut_ptr() as usize;
        let mut alloc = make_allocator(&[(base, 4096)]);
        let layout = Layout::from_size_align(64, 8).unwrap();
        let result = unsafe { alloc.allocate(layout, BlockOwner::Kernel) };
        assert!(result.is_ok());
    }

    #[test]
    fn allocate_with_task_owner_returns_ok() {
        let mut memory = vec![0u8; 4096];
        let base = memory.as_mut_ptr() as usize;
        let mut alloc = make_allocator(&[(base, 4096)]);
        let layout = Layout::from_size_align(64, 8).unwrap();
        let result = unsafe { alloc.allocate(layout, BlockOwner::Task(1)) };
        assert!(result.is_ok());
    }

    #[test]
    fn deallocate_by_owner_frees_task_blocks() {
        let region_size = 2 * needed_for(64);
        let mut memory = vec![0u8; region_size];
        let base = memory.as_mut_ptr() as usize;
        let mut alloc = make_allocator(&[(base, region_size)]);
        let layout = Layout::from_size_align(64, 8).unwrap();
        unsafe { alloc.allocate(layout, BlockOwner::Task(42)) }.unwrap();
        unsafe { alloc.allocate(layout, BlockOwner::Kernel) }.unwrap();
        assert!(matches!(
            unsafe { alloc.allocate(layout, BlockOwner::Kernel) },
            Err(AllocError::OutOfMemory)
        ));
        unsafe { alloc.deallocate_by_owner(42) };
        assert!(unsafe { alloc.allocate(layout, BlockOwner::Kernel) }.is_ok());
    }

    #[test]
    fn deallocate_by_owner_does_not_free_kernel_blocks() {
        let region_size = 2 * needed_for(64);
        let mut memory = vec![0u8; region_size];
        let base = memory.as_mut_ptr() as usize;
        let mut alloc = make_allocator(&[(base, region_size)]);
        let layout = Layout::from_size_align(64, 8).unwrap();
        unsafe { alloc.allocate(layout, BlockOwner::Kernel) }.unwrap();
        unsafe { alloc.allocate(layout, BlockOwner::Kernel) }.unwrap();
        unsafe { alloc.deallocate_by_owner(99) };
        assert!(matches!(
            unsafe { alloc.allocate(layout, BlockOwner::Kernel) },
            Err(AllocError::OutOfMemory)
        ));
    }

    #[test]
    fn deallocate_by_owner_only_frees_matching_task() {
        let region_size = 3 * needed_for(64);
        let mut memory = vec![0u8; region_size];
        let base = memory.as_mut_ptr() as usize;
        let mut alloc = make_allocator(&[(base, region_size)]);
        let layout = Layout::from_size_align(64, 8).unwrap();
        unsafe { alloc.allocate(layout, BlockOwner::Task(1)) }.unwrap();
        unsafe { alloc.allocate(layout, BlockOwner::Task(2)) }.unwrap();
        unsafe { alloc.allocate(layout, BlockOwner::Task(1)) }.unwrap();
        unsafe { alloc.deallocate_by_owner(1) };
        assert!(unsafe { alloc.allocate(layout, BlockOwner::Kernel) }.is_ok());
        assert!(unsafe { alloc.allocate(layout, BlockOwner::Kernel) }.is_ok());
        assert!(matches!(
            unsafe { alloc.allocate(layout, BlockOwner::Kernel) },
            Err(AllocError::OutOfMemory)
        ));
    }
}
