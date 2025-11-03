use core::alloc::{GlobalAlloc, Layout};
use core::mem::MaybeUninit;
use core::sync::atomic::{AtomicUsize, Ordering};

#[global_allocator]
pub static MEMORY_ALLOCATOR: MemoryAllocator = MemoryAllocator::new();

trait Xpto {

}
pub struct MemoryAllocator {
   root: MaybeUninit<&'static (dyn GlobalAlloc + Sync)>,
   used: AtomicUsize
}

impl MemoryAllocator {
    const fn new() -> Self {
        MemoryAllocator {
            root: MaybeUninit::uninit(),
            used: AtomicUsize::new(0)
        }
    }

    pub unsafe fn init(&self, allocator: &'static (dyn GlobalAlloc + Sync)) {
        core::ptr::addr_of!(self.root)
            .cast_mut()
            .write(MaybeUninit::new(allocator));
    }

    pub fn used(&self) -> usize {
        self.used.load(Ordering::Relaxed)
    }

}

unsafe impl GlobalAlloc for MemoryAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        self.used.fetch_add(layout.size(), Ordering::Relaxed);
        self.root.assume_init().alloc(layout)
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        self.used.fetch_sub(layout.size(), Ordering::Relaxed);
        self.root.assume_init().dealloc(ptr, layout);
    }
}