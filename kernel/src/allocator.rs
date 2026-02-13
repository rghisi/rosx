#[cfg(not(test))]
use core::alloc::{GlobalAlloc, Layout};
#[cfg(not(test))]
use core::mem::MaybeUninit;
#[cfg(not(test))]
use core::sync::atomic::{AtomicUsize, Ordering};

#[cfg(not(test))]
use crate::kernel::KERNEL;

#[cfg(not(test))]
#[global_allocator]
pub static GLOBAL_ALLOCATOR: GlobalKernelAllocator = GlobalKernelAllocator;

#[cfg(not(test))]
pub struct GlobalKernelAllocator;

#[cfg(not(test))]
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

#[cfg(not(test))]
pub static MEMORY_ALLOCATOR: MemoryAllocator = MemoryAllocator::new();

#[cfg(not(test))]
pub struct MemoryAllocator {
    root: MaybeUninit<&'static (dyn GlobalAlloc + Sync)>,
    used: AtomicUsize,
}

#[cfg(not(test))]
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

#[cfg(not(test))]
unsafe impl Sync for MemoryAllocator {}

#[cfg(not(test))]
#[alloc_error_handler]
pub fn alloc_error_handler(layout: Layout) -> ! {
    panic!("allocation error: {:?}", layout)
}

#[cfg(not(test))]
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
