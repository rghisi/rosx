use core::alloc::Layout;
use core::ptr;
use core::sync::atomic::{AtomicUsize, Ordering};

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
const BACKREF_BYTES: usize = core::mem::size_of::<usize>();

#[derive(Debug, PartialEq)]
pub(crate) enum AllocError {
    OutOfMemory,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub(crate) struct FreeListStats {
    pub free_bytes: usize,
    pub free_block_count: usize,
    pub largest_free_block: usize,
}

pub struct FreeListAllocator {
    head: *mut FreeBlock,
    alloc_head: *mut AllocHeader,
    regions: MemoryBlocks,
}

fn align_up(addr: usize, align: usize) -> usize {
    (addr + align - 1) & !(align - 1)
}

static BAD_FREE_COUNT: AtomicUsize = AtomicUsize::new(0);
const BAD_FREE_PRINT_LIMIT: usize = 10;

fn bad_free(reason: &'static str, p: usize, size: usize) {
    let count = BAD_FREE_COUNT.fetch_add(1, Ordering::Relaxed);
    #[cfg(not(test))]
    if count < BAD_FREE_PRINT_LIMIT {
        crate::kprintln!("BAD-FREE {} p=0x{:x} sz=0x{:x}", reason, p, size);
    }
    #[cfg(test)]
    let _ = (reason, count);
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
        FreeListAllocator {
            head,
            alloc_head: ptr::null_mut(),
            regions: *memory_blocks,
        }
    }

    pub unsafe fn allocate(&mut self, layout: Layout, owner: BlockOwner) -> Result<*mut u8, AllocError> {
        if layout.size() == 0 {
            return Err(AllocError::OutOfMemory);
        }

        let align = layout.align();
        let usable = align_up(layout.size(), BLOCK_ALIGN);
        let slack = if align > BLOCK_ALIGN {
            align + BACKREF_BYTES
        } else {
            0
        };
        let needed = (ALLOC_HDR + usable + slack).max(BLOCK_HDR);

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
                if align <= BLOCK_ALIGN {
                    return Ok((start + ALLOC_HDR) as *mut u8);
                }
                let aligned = align_up(start + ALLOC_HDR + BACKREF_BYTES, align);
                let backref = aligned - BACKREF_BYTES;
                unsafe {
                    (backref as *mut usize).write(backref - start);
                }
                return Ok(aligned as *mut u8);
            }
            prev_next = &mut (*current).next;
            current = (*current).next;
        }
        Err(AllocError::OutOfMemory)
    }

    unsafe fn block_start(&self, ptr: usize, align: usize) -> Option<usize> {
        if align <= BLOCK_ALIGN {
            let start = ptr.checked_sub(ALLOC_HDR);
            if start.is_none() {
                bad_free("invalid-pointer", ptr, 0);
            }
            return start;
        }
        let Some(backref_addr) = ptr.checked_sub(BACKREF_BYTES) else {
            bad_free("invalid-pointer", ptr, 0);
            return None;
        };
        if !self.region_contains(backref_addr, BACKREF_BYTES) {
            bad_free("outside-regions", ptr, 0);
            return None;
        }
        let offset = unsafe { (backref_addr as *const usize).read_unaligned() };
        let start = backref_addr.checked_sub(offset);
        if start.is_none() {
            bad_free("bogus-backref", ptr, 0);
        }
        start
    }

    pub unsafe fn deallocate(&mut self, ptr: *mut u8, align: usize) {
        let Some(start) = self.block_start(ptr as usize, align) else {
            return;
        };
        if start % BLOCK_ALIGN != 0 {
            bad_free("unaligned-pointer", ptr as usize, 0);
            return;
        }
        if !self.region_contains(start, ALLOC_HDR) {
            bad_free("outside-regions", ptr as usize, 0);
            return;
        }
        let header = start as *mut AllocHeader;
        let block_size = (*header).size;
        if block_size < ALLOC_HDR || !self.region_contains(start, block_size) {
            bad_free("insane-header-size", ptr as usize, block_size);
            return;
        }

        let mut prev_next: *mut *mut AllocHeader = &mut self.alloc_head;
        let mut current = self.alloc_head;
        while !current.is_null() && current != header {
            prev_next = &mut (*current).next;
            current = (*current).next;
        }
        if current.is_null() {
            bad_free("not-in-alloc-list", ptr as usize, block_size);
            return;
        }
        *prev_next = (*header).next;

        self.insert_free_block(start, block_size);
    }

    fn region_contains(&self, start: usize, size: usize) -> bool {
        let Some(end) = start.checked_add(size) else {
            return false;
        };
        self.regions.blocks[..self.regions.count]
            .iter()
            .any(|region| {
                region.size > 0
                    && start >= region.start
                    && end <= region.start.saturating_add(region.size)
            })
    }

    pub(crate) fn stats(&self) -> FreeListStats {
        let mut free_bytes = 0usize;
        let mut free_block_count = 0usize;
        let mut largest_free_block = 0usize;
        let mut current = self.head;
        while !current.is_null() {
            let size = unsafe { (*current).size };
            free_bytes += size;
            free_block_count += 1;
            largest_free_block = largest_free_block.max(size);
            current = unsafe { (*current).next };
        }
        FreeListStats {
            free_bytes,
            free_block_count,
            largest_free_block,
        }
    }

    pub unsafe fn deallocate_by_owner(&mut self, task_id: usize) -> usize {
        let target = BlockOwner::Task(task_id);
        let mut freed_usable = 0usize;
        let mut prev_next: *mut *mut AllocHeader = &mut self.alloc_head;
        let mut current = self.alloc_head;
        while !current.is_null() {
            let next = (*current).next;
            if (*current).owner == target {
                *prev_next = next;
                let start = current as usize;
                let block_size = (*current).size;
                freed_usable += block_size.saturating_sub(ALLOC_HDR);
                self.insert_free_block(start, block_size);
            } else {
                prev_next = &mut (*current).next;
            }
            current = next;
        }
        freed_usable
    }

    unsafe fn insert_free_block(&mut self, start: usize, block_size: usize) -> bool {
        if block_size < BLOCK_HDR {
            bad_free("size-below-header", start, block_size);
            return false;
        }
        let Some(end) = start.checked_add(block_size) else {
            bad_free("size-overflow", start, block_size);
            return false;
        };
        let mut probe = self.head;
        while !probe.is_null() {
            let probe_start = probe as usize;
            let probe_size = unsafe { (*probe).size };
            if start < probe_start.saturating_add(probe_size) && probe_start < end {
                bad_free("overlaps-free-entry", start, block_size);
                return false;
            }
            probe = unsafe { (*probe).next };
        }

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
        true
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
        unsafe { alloc.deallocate(first, BLOCK_ALIGN) };
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
    fn allocation_beyond_block_alignment_succeeds() {
        let mut memory = vec![0u8; 4096];
        let base = memory.as_mut_ptr() as usize;
        let mut alloc = make_allocator(&[(base, 4096)]);
        let layout = Layout::from_size_align(64, BLOCK_ALIGN * 2).unwrap();
        let ptr = unsafe { alloc.allocate(layout, BlockOwner::Kernel) }.unwrap();
        assert_eq!(ptr as usize % (BLOCK_ALIGN * 2), 0);
    }

    #[test]
    fn overaligned_allocations_honor_alignment_and_free_clean() {
        let mut memory = vec![0u8; 16 * 1024];
        let base = memory.as_mut_ptr() as usize;
        let mut alloc = make_allocator(&[(base, 16 * 1024)]);
        let bad_before = BAD_FREE_COUNT.load(Ordering::Relaxed);
        for align in [16usize, 32, 64, 4096] {
            let layout = Layout::from_size_align(48, align).unwrap();
            let ptr = unsafe { alloc.allocate(layout, BlockOwner::Kernel) }.unwrap();
            assert_eq!(ptr as usize % align, 0);
            unsafe { alloc.deallocate(ptr, align) };
        }
        assert_eq!(BAD_FREE_COUNT.load(Ordering::Relaxed), bad_before);
        let entries = free_entries(&alloc);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].1, 16 * 1024);
    }

    #[test]
    fn overaligned_dealloc_coalesces_with_kernel_neighbors() {
        let mut memory = vec![0u8; 4096];
        let base = memory.as_mut_ptr() as usize;
        let mut alloc = make_allocator(&[(base, 4096)]);
        let small = Layout::from_size_align(64, 8).unwrap();
        let wide = Layout::from_size_align(64, 16).unwrap();
        let first = unsafe { alloc.allocate(small, BlockOwner::Kernel) }.unwrap();
        let middle = unsafe { alloc.allocate(wide, BlockOwner::Task(3)) }.unwrap();
        let last = unsafe { alloc.allocate(small, BlockOwner::Kernel) }.unwrap();
        assert_eq!(middle as usize % 16, 0);
        let bad_before = BAD_FREE_COUNT.load(Ordering::Relaxed);
        unsafe { alloc.deallocate(middle, 16) };
        unsafe { alloc.deallocate(first, BLOCK_ALIGN) };
        unsafe { alloc.deallocate(last, BLOCK_ALIGN) };
        assert_eq!(BAD_FREE_COUNT.load(Ordering::Relaxed), bad_before);
        let entries = free_entries(&alloc);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].1, 4096);
    }

    #[test]
    fn deallocate_rejects_fabricated_overaligned_pointer() {
        let mut memory = vec![0u8; 4096];
        let base = memory.as_mut_ptr() as usize;
        let mut alloc = make_allocator(&[(base, 4096)]);
        let layout = Layout::from_size_align(64, 16).unwrap();
        let real = unsafe { alloc.allocate(layout, BlockOwner::Kernel) }.unwrap();
        let free_before = free_entries(&alloc);
        let bad_before = BAD_FREE_COUNT.load(Ordering::Relaxed);
        unsafe { alloc.deallocate((real as usize + 48) as *mut u8, 16) };
        assert_eq!(BAD_FREE_COUNT.load(Ordering::Relaxed), bad_before + 1);
        assert_eq!(free_entries(&alloc), free_before);
        unsafe { alloc.deallocate(real, 16) };
        assert_eq!(BAD_FREE_COUNT.load(Ordering::Relaxed), bad_before + 1);
        assert_eq!(free_entries(&alloc).len(), 1);
    }

    #[test]
    fn deallocate_rejects_overaligned_pointer_outside_regions() {
        let mut memory = vec![0u8; 4096];
        let base = memory.as_mut_ptr() as usize;
        let mut alloc = make_allocator(&[(base, 4096)]);
        #[repr(align(64))]
        struct Foreign([usize; 8]);
        let foreign = Foreign([0; 8]);
        let bogus = core::ptr::addr_of!(foreign).cast::<u8>() as usize;
        let bad_before = BAD_FREE_COUNT.load(Ordering::Relaxed);
        unsafe { alloc.deallocate(bogus as *mut u8, 16) };
        assert_eq!(BAD_FREE_COUNT.load(Ordering::Relaxed), bad_before + 1);
        assert_eq!(free_entries(&alloc).len(), 1);
    }

    #[test]
    fn absurd_alignment_returns_out_of_memory() {
        let mut memory = vec![0u8; 4096];
        let base = memory.as_mut_ptr() as usize;
        let mut alloc = make_allocator(&[(base, 4096)]);
        let beyond_region = Layout::from_size_align(16, 1usize << 40).unwrap();
        assert!(matches!(
            unsafe { alloc.allocate(beyond_region, BlockOwner::Kernel) },
            Err(AllocError::OutOfMemory)
        ));
        assert_eq!(free_entries(&alloc).len(), 1);
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
        unsafe { alloc.deallocate(a, BLOCK_ALIGN) };
        unsafe { alloc.deallocate(b, BLOCK_ALIGN) };
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

    #[test]
    fn deallocate_by_owner_returns_total_freed_usable_bytes() {
        let region_size = 3 * needed_for(64);
        let mut memory = vec![0u8; region_size];
        let base = memory.as_mut_ptr() as usize;
        let mut alloc = make_allocator(&[(base, region_size)]);
        let layout = Layout::from_size_align(64, 8).unwrap();
        unsafe { alloc.allocate(layout, BlockOwner::Task(7)) }.unwrap();
        unsafe { alloc.allocate(layout, BlockOwner::Kernel) }.unwrap();
        unsafe { alloc.allocate(layout, BlockOwner::Task(7)) }.unwrap();
        assert_eq!(unsafe { alloc.deallocate_by_owner(7) }, 2 * 64);
        assert_eq!(unsafe { alloc.deallocate_by_owner(7) }, 0);
    }

    fn free_entries(alloc: &FreeListAllocator) -> Vec<(usize, usize)> {
        let mut entries = Vec::new();
        let mut current = alloc.head;
        while !current.is_null() {
            unsafe {
                entries.push((current as usize, (*current).size));
                current = (*current).next;
            }
        }
        entries
    }

    fn alloc_entries(alloc: &FreeListAllocator) -> Vec<usize> {
        let mut entries = Vec::new();
        let mut current = alloc.alloc_head;
        while !current.is_null() {
            unsafe {
                entries.push(current as usize);
                current = (*current).next;
            }
        }
        entries
    }

    fn emptied_allocator(region_size: usize) -> (Vec<u8>, FreeListAllocator, usize) {
        let mut memory = vec![0u8; region_size];
        let base = memory.as_mut_ptr() as usize;
        let aligned = align_up(base, BLOCK_ALIGN);
        let mut alloc = make_allocator(&[(base, region_size)]);
        let whole = Layout::from_size_align(region_size - (aligned - base) - ALLOC_HDR, 8).unwrap();
        let _big = unsafe { alloc.allocate(whole, BlockOwner::Kernel) }.unwrap();
        assert!(free_entries(&alloc).is_empty());
        (memory, alloc, aligned)
    }

    #[test]
    fn deallocate_rejects_null_pointer() {
        let mut memory = vec![0u8; 4096];
        let base = memory.as_mut_ptr() as usize;
        let mut alloc = make_allocator(&[(base, 4096)]);
        let bad_before = BAD_FREE_COUNT.load(Ordering::Relaxed);
        unsafe { alloc.deallocate(ptr::null_mut(), BLOCK_ALIGN) };
        assert_eq!(BAD_FREE_COUNT.load(Ordering::Relaxed), bad_before + 1);
        assert_eq!(alloc_entries(&alloc), Vec::<usize>::new());
    }

    #[test]
    fn deallocate_rejects_unaligned_pointer_inside_region() {
        let mut memory = vec![0u8; 4096];
        let base = memory.as_mut_ptr() as usize;
        let mut alloc = make_allocator(&[(base, 4096)]);
        let bogus = (align_up(base, BLOCK_ALIGN) + ALLOC_HDR + 1) as *mut u8;
        let free_before = free_entries(&alloc);
        let bad_before = BAD_FREE_COUNT.load(Ordering::Relaxed);
        unsafe { alloc.deallocate(bogus, BLOCK_ALIGN) };
        assert_eq!(BAD_FREE_COUNT.load(Ordering::Relaxed), bad_before + 1);
        assert_eq!(free_entries(&alloc), free_before);
    }

    #[test]
    fn deallocate_rejects_pointer_outside_regions() {
        let mut memory = vec![0u8; 4096];
        let base = memory.as_mut_ptr() as usize;
        let mut alloc = make_allocator(&[(base, 4096)]);
        #[repr(align(64))]
        struct Foreign([usize; 8]);
        let foreign = Foreign([0; 8]);
        let bogus = core::ptr::addr_of!(foreign).cast::<u8>() as usize;
        assert!(!alloc.region_contains(bogus, ALLOC_HDR));
        let free_before = free_entries(&alloc);
        let bad_before = BAD_FREE_COUNT.load(Ordering::Relaxed);
        unsafe { alloc.deallocate((bogus + ALLOC_HDR) as *mut u8, BLOCK_ALIGN) };
        assert_eq!(BAD_FREE_COUNT.load(Ordering::Relaxed), bad_before + 1);
        assert_eq!(free_entries(&alloc), free_before);
    }

    #[test]
    fn deallocate_rejects_double_free_and_keeps_lists_intact() {
        let mut memory = vec![0u8; 4096];
        let base = memory.as_mut_ptr() as usize;
        let mut alloc = make_allocator(&[(base, 4096)]);
        let layout = Layout::from_size_align(64, 8).unwrap();
        let first = unsafe { alloc.allocate(layout, BlockOwner::Kernel) }.unwrap();
        let _second = unsafe { alloc.allocate(layout, BlockOwner::Kernel) }.unwrap();
        unsafe { alloc.deallocate(first, BLOCK_ALIGN) };
        let free_before = free_entries(&alloc);
        let alloc_before = alloc_entries(&alloc);
        let bad_before = BAD_FREE_COUNT.load(Ordering::Relaxed);
        unsafe { alloc.deallocate(first, BLOCK_ALIGN) };
        assert_eq!(BAD_FREE_COUNT.load(Ordering::Relaxed), bad_before + 1);
        assert_eq!(free_entries(&alloc), free_before);
        assert_eq!(alloc_entries(&alloc), alloc_before);
        let first_start = first as usize - ALLOC_HDR;
        let duplicates = free_entries(&alloc)
            .iter()
            .filter(|(start, _)| *start == first_start)
            .count();
        assert_eq!(duplicates, 1);
        let recycled = unsafe { alloc.allocate(layout, BlockOwner::Kernel) }.unwrap();
        assert_eq!(recycled, first);
        unsafe { alloc.deallocate(recycled, BLOCK_ALIGN) };
        let free_after_recycle = free_entries(&alloc);
        let alloc_after_recycle = alloc_entries(&alloc);
        let bad_after_recycle = BAD_FREE_COUNT.load(Ordering::Relaxed);
        unsafe { alloc.deallocate(recycled, BLOCK_ALIGN) };
        assert_eq!(
            BAD_FREE_COUNT.load(Ordering::Relaxed),
            bad_after_recycle + 1
        );
        assert_eq!(free_entries(&alloc), free_after_recycle);
        assert_eq!(alloc_entries(&alloc), alloc_after_recycle);
        assert!(unsafe { alloc.allocate(layout, BlockOwner::Kernel) }.is_ok());
    }

    #[test]
    fn insert_free_block_rejects_size_below_header() {
        let (_memory, mut alloc, aligned) = emptied_allocator(4096);
        let bad_before = BAD_FREE_COUNT.load(Ordering::Relaxed);
        assert!(!unsafe { alloc.insert_free_block(aligned + 640, BLOCK_HDR - BLOCK_ALIGN) });
        assert_eq!(BAD_FREE_COUNT.load(Ordering::Relaxed), bad_before + 1);
        assert!(free_entries(&alloc).is_empty());
    }

    #[test]
    fn insert_free_block_rejects_duplicate_entry() {
        let (_memory, mut alloc, aligned) = emptied_allocator(4096);
        assert!(unsafe { alloc.insert_free_block(aligned + 256, 128) });
        let snapshot = free_entries(&alloc);
        let bad_before = BAD_FREE_COUNT.load(Ordering::Relaxed);
        assert!(!unsafe { alloc.insert_free_block(aligned + 256, 128) });
        assert_eq!(BAD_FREE_COUNT.load(Ordering::Relaxed), bad_before + 1);
        assert_eq!(free_entries(&alloc), snapshot);
    }

    #[test]
    fn insert_free_block_rejects_overlapping_entry() {
        let (_memory, mut alloc, aligned) = emptied_allocator(4096);
        assert!(unsafe { alloc.insert_free_block(aligned + 256, 128) });
        let snapshot = free_entries(&alloc);
        let bad_before = BAD_FREE_COUNT.load(Ordering::Relaxed);
        assert!(!unsafe { alloc.insert_free_block(aligned + 320, 128) });
        assert_eq!(BAD_FREE_COUNT.load(Ordering::Relaxed), bad_before + 1);
        assert_eq!(free_entries(&alloc), snapshot);
    }

    #[test]
    fn insert_free_block_accepts_exact_header_size() {
        let (_memory, mut alloc, aligned) = emptied_allocator(4096);
        assert!(unsafe { alloc.insert_free_block(aligned + 512, BLOCK_HDR) });
        assert_eq!(free_entries(&alloc), vec![(aligned + 512, BLOCK_HDR)]);
    }

    #[test]
    fn deallocate_coalesces_adjacent_blocks_into_single_entry() {
        let mut memory = vec![0u8; 4096];
        let base = memory.as_mut_ptr() as usize;
        let mut alloc = make_allocator(&[(base, 4096)]);
        let layout = Layout::from_size_align(64, 8).unwrap();
        let first = unsafe { alloc.allocate(layout, BlockOwner::Kernel) }.unwrap();
        let second = unsafe { alloc.allocate(layout, BlockOwner::Kernel) }.unwrap();
        let bad_before = BAD_FREE_COUNT.load(Ordering::Relaxed);
        unsafe { alloc.deallocate(first, BLOCK_ALIGN) };
        unsafe { alloc.deallocate(second, BLOCK_ALIGN) };
        assert_eq!(BAD_FREE_COUNT.load(Ordering::Relaxed), bad_before);
        let aligned = align_up(base, BLOCK_ALIGN);
        let entries = free_entries(&alloc);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].0, aligned);
        assert_eq!(entries[0].1, 4096 - (aligned - base));
        assert!(
            unsafe { alloc.allocate(Layout::from_size_align(80, 8).unwrap(), BlockOwner::Kernel) }
                .is_ok()
        );
    }

    #[test]
    fn alloc_dealloc_roundtrip_on_exact_fit_region() {
        let region_size = needed_for(8);
        let mut memory = vec![0u8; region_size];
        let base = memory.as_mut_ptr() as usize;
        let mut alloc = make_allocator(&[(base, region_size)]);
        let aligned = align_up(base, BLOCK_ALIGN);
        let layout = Layout::from_size_align(8, 8).unwrap();
        let ptr = unsafe { alloc.allocate(layout, BlockOwner::Kernel) }.unwrap();
        assert_eq!(ptr as usize, aligned + ALLOC_HDR);
        assert!(free_entries(&alloc).is_empty());
        let bad_before = BAD_FREE_COUNT.load(Ordering::Relaxed);
        unsafe { alloc.deallocate(ptr, BLOCK_ALIGN) };
        assert_eq!(BAD_FREE_COUNT.load(Ordering::Relaxed), bad_before);
        let entries = free_entries(&alloc);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].1, region_size - (aligned - base));
    }

    #[test]
    fn stats_of_fresh_allocator_reports_the_whole_region_as_free() {
        let mut memory = vec![0u8; 4096];
        let base = memory.as_mut_ptr() as usize;
        let aligned = align_up(base, BLOCK_ALIGN);
        let alloc = make_allocator(&[(base, 4096)]);
        let stats = alloc.stats();
        assert_eq!(stats.free_block_count, 1);
        assert_eq!(stats.free_bytes, 4096 - (aligned - base));
        assert_eq!(stats.largest_free_block, 4096 - (aligned - base));
    }

    #[test]
    fn stats_count_each_region_as_a_free_entry() {
        let mut memory1 = vec![0u8; 1024];
        let mut memory2 = vec![0u8; 2048];
        let base1 = memory1.as_mut_ptr() as usize;
        let base2 = memory2.as_mut_ptr() as usize;
        let aligned1 = align_up(base1, BLOCK_ALIGN);
        let aligned2 = align_up(base2, BLOCK_ALIGN);
        let alloc = make_allocator(&[(base1, 1024), (base2, 2048)]);
        let stats = alloc.stats();
        assert_eq!(stats.free_block_count, 2);
        assert_eq!(
            stats.free_bytes,
            (1024 - (aligned1 - base1)) + (2048 - (aligned2 - base2))
        );
        assert_eq!(stats.largest_free_block, 2048 - (aligned2 - base2));
    }

    #[test]
    fn stats_shrink_on_allocation_and_grow_back_on_deallocation() {
        let mut memory = vec![0u8; 4096];
        let base = memory.as_mut_ptr() as usize;
        let mut alloc = make_allocator(&[(base, 4096)]);
        let fresh = alloc.stats();
        let layout = Layout::from_size_align(64, 8).unwrap();
        let ptr = unsafe { alloc.allocate(layout, BlockOwner::Kernel) }.unwrap();
        let after_alloc = alloc.stats();
        assert_eq!(after_alloc.free_block_count, 1);
        assert_eq!(after_alloc.free_bytes, fresh.free_bytes - needed_for(64));
        assert_eq!(after_alloc.largest_free_block, after_alloc.free_bytes);
        unsafe { alloc.deallocate(ptr, BLOCK_ALIGN) };
        assert_eq!(alloc.stats(), fresh);
    }

    #[test]
    fn stats_of_exhausted_allocator_are_zero() {
        let (_memory, alloc, _aligned) = emptied_allocator(4096);
        let stats = alloc.stats();
        assert_eq!(stats.free_bytes, 0);
        assert_eq!(stats.free_block_count, 0);
        assert_eq!(stats.largest_free_block, 0);
    }

    #[test]
    fn stats_report_fragmentation_after_releasing_a_middle_block() {
        let region_size = 4 * needed_for(64);
        let mut memory = vec![0u8; region_size];
        let base = memory.as_mut_ptr() as usize;
        let mut alloc = make_allocator(&[(base, region_size)]);
        let layout = Layout::from_size_align(64, 8).unwrap();
        let _a = unsafe { alloc.allocate(layout, BlockOwner::Kernel) }.unwrap();
        let b = unsafe { alloc.allocate(layout, BlockOwner::Kernel) }.unwrap();
        let _c = unsafe { alloc.allocate(layout, BlockOwner::Kernel) }.unwrap();
        unsafe { alloc.deallocate(b, BLOCK_ALIGN) };
        let stats = alloc.stats();
        assert_eq!(stats.free_block_count, 2);
        assert_eq!(stats.free_bytes, 2 * needed_for(64));
        assert_eq!(stats.largest_free_block, needed_for(64));
    }
}
