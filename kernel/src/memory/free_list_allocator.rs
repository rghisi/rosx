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
        (*block).next = self.head;
        self.head = block;
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
}
