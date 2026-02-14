use core::alloc::{GlobalAlloc, Layout};
use core::sync::atomic::{AtomicUsize, Ordering};

use crate::kernel::KERNEL;
use crate::once::Once;

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
    root: Once<&'static (dyn GlobalAlloc + Sync)>,
    used: AtomicUsize,
}

impl MemoryAllocator {
    const fn new() -> Self {
        MemoryAllocator {
            root: Once::new(),
            used: AtomicUsize::new(0),
        }
    }

    pub fn init(&self, allocator: &'static (dyn GlobalAlloc + Sync)) {
        self.root.call_once(|| allocator);
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
        unsafe {
            self.used.fetch_add(layout.size(), Ordering::Relaxed);
            self.root.get().unwrap().alloc(layout)
        }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe {
            self.used.fetch_sub(layout.size(), Ordering::Relaxed);
            self.root.get().unwrap().dealloc(ptr, layout);
        }
    }
}
