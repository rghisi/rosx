use core::alloc::Layout;

use crate::memory::free_list_allocator::{BlockOwner, FreeListAllocator};
use crate::memory::slab_heap::{SlabHeap, realloc_same_class, small_class};

pub trait KernelMemoryAllocator {
    fn allocate(&mut self, tier0: &mut FreeListAllocator, layout: Layout) -> *mut u8;

    fn deallocate(&mut self, tier0: &mut FreeListAllocator, pointer: *mut u8, layout: Layout);

    fn reallocate(
        &mut self,
        tier0: &mut FreeListAllocator,
        pointer: *mut u8,
        layout: Layout,
        new_size: usize,
    ) -> *mut u8;
}

pub struct KernelSlabAllocator {
    slab: SlabHeap,
}

impl KernelSlabAllocator {
    pub const fn new() -> Self {
        KernelSlabAllocator {
            slab: SlabHeap::new(),
        }
    }
}

impl KernelMemoryAllocator for KernelSlabAllocator {
    /// Requests that `small_class` covers are served from slab arenas grown
    /// out of Tier 0; every other request passes through to Tier 0 as a
    /// `BlockOwner::Kernel` block. See docs/rationale/kernel-slab-heap.md.
    fn allocate(&mut self, tier0: &mut FreeListAllocator, layout: Layout) -> *mut u8 {
        if small_class(&layout).is_none() {
            return unsafe { tier0.allocate(layout, BlockOwner::Kernel) }.unwrap_or(core::ptr::null_mut());
        }
        unsafe { self.slab.allocate(tier0, layout) }
    }

    /// Free routes by the same `small_class` predicate and nothing else:
    /// covered blocks belong to the slab, everything else to Tier 0.
    /// See docs/rationale/kernel-slab-heap.md.
    fn deallocate(&mut self, tier0: &mut FreeListAllocator, pointer: *mut u8, layout: Layout) {
        if small_class(&layout).is_some() {
            unsafe { self.slab.deallocate(tier0, pointer, layout) };
        } else {
            unsafe { tier0.deallocate(pointer, layout.align()) };
        }
    }

    /// Same-class growth keeps the pointer; every other case runs an
    /// alloc-copy-free through this policy, so both ends follow the same
    /// routing. See docs/rationale/kernel-slab-heap.md.
    fn reallocate(
        &mut self,
        tier0: &mut FreeListAllocator,
        pointer: *mut u8,
        layout: Layout,
        new_size: usize,
    ) -> *mut u8 {
        if realloc_same_class(&layout, new_size) {
            return pointer;
        }
        let Ok(new_layout) = Layout::from_size_align(new_size, layout.align()) else {
            return core::ptr::null_mut();
        };
        let new_pointer = self.allocate(tier0, new_layout);
        if !new_pointer.is_null() {
            unsafe { core::ptr::copy_nonoverlapping(pointer, new_pointer, layout.size().min(new_size)) };
            self.deallocate(tier0, pointer, layout);
        }
        new_pointer
    }
}
