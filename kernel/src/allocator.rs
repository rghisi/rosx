use core::alloc::{GlobalAlloc, Layout};
use core::sync::atomic::{AtomicUsize, Ordering};

use buddy_system_allocator::LockedHeap;

use crate::kernel::KERNEL;

pub const MAX_MEMORY_BLOCKS: usize = 32;

pub struct MemoryBlock {
    pub start: usize,
    pub size: usize,
}

pub struct MemoryBlocks {
    pub blocks: [MemoryBlock; MAX_MEMORY_BLOCKS],
    pub count: usize,
}

#[cfg_attr(not(test), global_allocator)]
pub static GLOBAL_ALLOCATOR: GlobalKernelAllocator = GlobalKernelAllocator;

pub struct GlobalKernelAllocator;

unsafe impl GlobalAlloc for GlobalKernelAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        unsafe {
            if !KERNEL.is_null() {
                (*KERNEL).alloc(layout)
            } else {
                MEMORY_ALLOCATOR.alloc(layout)
            }
        }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe {
            if !KERNEL.is_null() {
                (*KERNEL).dealloc(ptr, layout)
            } else {
                MEMORY_ALLOCATOR.dealloc(ptr, layout)
            }
        }
    }
}

pub static MEMORY_ALLOCATOR: MemoryAllocator = MemoryAllocator::new();

pub struct MemoryAllocator {
    heap: LockedHeap<27>,
    used: AtomicUsize,
}

impl MemoryAllocator {
    const fn new() -> Self {
        MemoryAllocator {
            heap: LockedHeap::<27>::new(),
            used: AtomicUsize::new(0),
        }
    }

    pub fn init(&self, memory_blocks: &MemoryBlocks) {
        for i in 0..memory_blocks.count {
            let block = &memory_blocks.blocks[i];
            unsafe {
                self.heap.lock().init(block.start, block.size);
            }
        }
    }

    pub fn used(&self) -> usize {
        self.used.load(Ordering::Relaxed)
    }
}

#[cfg_attr(not(test), alloc_error_handler)]
pub fn alloc_error_handler(layout: Layout) -> ! {
    panic!("allocation error: {:?}", layout)
}

unsafe impl GlobalAlloc for MemoryAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        self.used.fetch_add(layout.size(), Ordering::Relaxed);
        unsafe { self.heap.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        self.used.fetch_sub(layout.size(), Ordering::Relaxed);
        unsafe { self.heap.dealloc(ptr, layout) };
    }
}
