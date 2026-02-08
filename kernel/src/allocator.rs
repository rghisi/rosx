use core::alloc::{GlobalAlloc, Layout};
use core::mem::MaybeUninit;
use core::sync::atomic::{AtomicUsize, Ordering};

use crate::kernel::KERNEL;

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

trait Xpto {}
pub struct MemoryAllocator {
    root: MaybeUninit<&'static (dyn GlobalAlloc + Sync)>,
    used: AtomicUsize,
}

impl MemoryAllocator {
    const fn new() -> Self {
        MemoryAllocator {
            root: MaybeUninit::uninit(),
            used: AtomicUsize::new(0),
        }
    }

    pub unsafe fn init(&self, allocator: &'static (dyn GlobalAlloc + Sync)) {
        unsafe {
            core::ptr::addr_of!(self.root)
                .cast_mut()
                .write(MaybeUninit::new(allocator));
        }
    }

    pub fn used(&self) -> usize {
        self.used.load(Ordering::Relaxed)
    }
}

unsafe impl Sync for MemoryAllocator {}

#[cfg_attr(not(test), alloc_error_handler)]
pub fn alloc_error_handler(layout: Layout) -> ! {
    panic!("allocation error: {:?}", layout)
}

unsafe impl GlobalAlloc for MemoryAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        unsafe {
            self.used.fetch_add(layout.size(), Ordering::Relaxed);
            self.root.assume_init().alloc(layout)
        }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe {
            self.used.fetch_sub(layout.size(), Ordering::Relaxed);
            self.root.assume_init().dealloc(ptr, layout);
        }
    }
}
