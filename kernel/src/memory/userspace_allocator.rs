use core::alloc::Layout;
use super::bitmap_chunk_allocator::{ChunkAllocator, ChunkOwner};

pub const RESERVE_THRESHOLD: usize = 1;

struct FreeNode {
    next: *mut FreeNode,
    size: usize,
}

struct BlockHeader {
    next: *mut BlockHeader,
    chunk_count: usize,
    chunk_size: usize,
    allocation_count: usize,
    free_list: *mut FreeNode,
}

pub struct UserSpaceAllocator {
    chunk_allocator: *mut dyn ChunkAllocator,
    block_list: *mut BlockHeader,
    free_block_count: usize,
}

fn align_up(value: usize, alignment: usize) -> usize {
    (value + alignment - 1) & !(alignment - 1)
}

impl UserSpaceAllocator {
    pub fn new(chunk_allocator: *mut dyn ChunkAllocator) -> Self {
        Self {
            chunk_allocator,
            block_list: core::ptr::null_mut(),
            free_block_count: 0,
        }
    }

    pub fn alloc(&mut self, layout: Layout) -> *mut u8 {
        if layout.size() == 0 {
            return core::ptr::null_mut();
        }

        let mut block = self.block_list;
        while !block.is_null() {
            // Safety: block points to a BlockHeader written by a previous alloc_new_block call.
            let block_ref = unsafe { &mut *block };
            if let Some(ptr) = Self::alloc_from_block(block_ref, layout) {
                if block_ref.allocation_count == 0 {
                    self.free_block_count -= 1;
                }
                block_ref.allocation_count += 1;
                return ptr;
            }
            block = block_ref.next;
        }

        self.alloc_new_block(layout)
    }

    fn alloc_from_block(block: &mut BlockHeader, layout: Layout) -> Option<*mut u8> {
        let free_node_min = core::mem::size_of::<FreeNode>();
        let mut prev_next: *mut *mut FreeNode = &mut block.free_list;
        let mut node = block.free_list;

        while !node.is_null() {
            let node_addr = node as usize;
            // Safety: node is a valid FreeNode pointer written by us during init or dealloc.
            let (node_size, node_next) = unsafe { ((*node).size, (*node).next) };

            let alloc_start = align_up(node_addr, layout.align());
            let padding = alloc_start - node_addr;

            if padding + layout.size() <= node_size {
                let trail_addr = alloc_start + layout.size();
                let trail_end = node_addr + node_size;
                let trail_size = trail_end.saturating_sub(trail_addr);

                let trail: *mut FreeNode = if trail_size >= free_node_min {
                    // Safety: trail_addr is within the current free span.
                    unsafe {
                        let t = trail_addr as *mut FreeNode;
                        *t = FreeNode { next: node_next, size: trail_size };
                        t
                    }
                } else {
                    node_next
                };

                if padding >= free_node_min {
                    // Keep the leading fragment: shrink the existing node and re-point its next.
                    // Safety: node is the current valid FreeNode.
                    unsafe {
                        (*node).size = padding;
                        (*node).next = trail;
                    }
                } else {
                    // No usable leading fragment: splice out the current node.
                    // Safety: prev_next holds the address of the pointer that refers to node.
                    unsafe { *prev_next = trail; }
                }

                return Some(alloc_start as *mut u8);
            }

            // Safety: node is valid; advance both the cursor and the back-pointer.
            unsafe {
                prev_next = &mut (*node).next;
                node = node_next;
            }
        }

        None
    }

    fn alloc_new_block(&mut self, layout: Layout) -> *mut u8 {
        let header_size = core::mem::size_of::<BlockHeader>();
        let request = match Layout::from_size_align(layout.size() + header_size, 1) {
            Ok(l) => l,
            Err(_) => return core::ptr::null_mut(),
        };

        // Safety: chunk_allocator is a valid pointer supplied during construction.
        let allocation = match unsafe { (*self.chunk_allocator).allocate(request, ChunkOwner::Kernel) } {
            Some(a) => a,
            None => return core::ptr::null_mut(),
        };

        let block_start = allocation.ptr as usize;
        let total_bytes = allocation.chunk_count * allocation.chunk_size;
        let free_node_addr = block_start + header_size;
        let free_node_size = total_bytes - header_size;

        // Safety: both addresses are within the freshly allocated chunk.
        unsafe {
            let free_node = free_node_addr as *mut FreeNode;
            *free_node = FreeNode { next: core::ptr::null_mut(), size: free_node_size };

            let block_header = block_start as *mut BlockHeader;
            *block_header = BlockHeader {
                next: self.block_list,
                chunk_count: allocation.chunk_count,
                chunk_size: allocation.chunk_size,
                allocation_count: 0,
                free_list: free_node,
            };
            self.block_list = block_header;
        }

        // Safety: block_list now points to the BlockHeader we just wrote.
        let block_ref = unsafe { &mut *self.block_list };
        match Self::alloc_from_block(block_ref, layout) {
            Some(ptr) => {
                block_ref.allocation_count += 1;
                ptr
            }
            None => core::ptr::null_mut(),
        }
    }

    pub fn dealloc(&mut self, ptr: *mut u8, layout: Layout) {
        let ptr_addr = ptr as usize;

        let mut block = self.block_list;
        while !block.is_null() {
            // Safety: block points to a BlockHeader we wrote during alloc_new_block.
            let block_ref = unsafe { &mut *block };
            let block_start = block as usize;
            let block_end = block_start + block_ref.chunk_count * block_ref.chunk_size;

            if ptr_addr >= block_start && ptr_addr < block_end {
                Self::dealloc_from_block(block_ref, ptr, layout.size());
                block_ref.allocation_count -= 1;
                if block_ref.allocation_count == 0 {
                    self.free_block_count += 1;
                }
                return;
            }

            block = block_ref.next;
        }
    }

    fn dealloc_from_block(block: &mut BlockHeader, ptr: *mut u8, size: usize) {
        let ptr_addr = ptr as usize;
        let mut prev: *mut *mut FreeNode = &mut block.free_list;
        let mut node = block.free_list;

        while !node.is_null() {
            // Safety: node is a valid FreeNode pointer.
            if ptr_addr < node as usize {
                break;
            }
            unsafe {
                prev = &mut (*node).next;
                node = (*node).next;
            }
        }

        // Safety: ptr points to memory we previously allocated; size bytes are available there.
        unsafe {
            let new_node = ptr as *mut FreeNode;
            *new_node = FreeNode { next: node, size };
            *prev = new_node;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::bitmap_chunk_allocator::BitmapChunkAllocator;
    use core::alloc::Layout;

    fn make_chunk_alloc(chunk_size: usize, memory: &mut Vec<u8>) -> BitmapChunkAllocator {
        let base = memory.as_mut_ptr() as usize;
        BitmapChunkAllocator::with_chunk_size(chunk_size, &[(base, memory.len())])
    }

    #[test]
    fn alloc_returns_non_null_for_small_allocation() {
        let chunk_size = 4096;
        let mut memory = vec![0u8; 10 * chunk_size];
        let mut chunk_alloc = make_chunk_alloc(chunk_size, &mut memory);
        let mut allocator = UserSpaceAllocator::new(&mut chunk_alloc as *mut dyn ChunkAllocator);

        let layout = Layout::from_size_align(64, 8).unwrap();
        let ptr = allocator.alloc(layout);

        assert!(!ptr.is_null());
    }

    #[test]
    fn dealloc_returns_space_for_reuse() {
        let chunk_size = 4096;
        let mut memory = vec![0u8; 10 * chunk_size];
        let mut chunk_alloc = make_chunk_alloc(chunk_size, &mut memory);
        let mut allocator = UserSpaceAllocator::new(&mut chunk_alloc as *mut dyn ChunkAllocator);

        let layout = Layout::from_size_align(64, 8).unwrap();
        let ptr_a = allocator.alloc(layout);
        let ptr_b = allocator.alloc(layout);

        assert!(!ptr_a.is_null());
        assert!(!ptr_b.is_null());

        allocator.dealloc(ptr_a, layout);

        let ptr_c = allocator.alloc(layout);
        assert_eq!(ptr_c, ptr_a);
    }

    #[test]
    fn two_allocations_from_same_block_do_not_overlap() {
        let chunk_size = 4096;
        let mut memory = vec![0u8; 10 * chunk_size];
        let mut chunk_alloc = make_chunk_alloc(chunk_size, &mut memory);
        let mut allocator = UserSpaceAllocator::new(&mut chunk_alloc as *mut dyn ChunkAllocator);

        let layout = Layout::from_size_align(64, 8).unwrap();
        let ptr1 = allocator.alloc(layout);
        let ptr2 = allocator.alloc(layout);

        assert!(!ptr1.is_null());
        assert!(!ptr2.is_null());
        assert_ne!(ptr1, ptr2);

        unsafe {
            core::ptr::write_bytes(ptr1, 0xAA, 64);
            core::ptr::write_bytes(ptr2, 0xBB, 64);
            assert!(core::slice::from_raw_parts(ptr1, 64).iter().all(|&b| b == 0xAA));
            assert!(core::slice::from_raw_parts(ptr2, 64).iter().all(|&b| b == 0xBB));
        }
    }
}
