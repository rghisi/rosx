use core::alloc::{GlobalAlloc, Layout};
use core::mem::MaybeUninit;
use core::ptr::{NonNull, null_mut};
use core::sync::atomic::{AtomicUsize, Ordering};

use buddy_system_allocator::LockedHeap;
use system::memory::MemoryRegion;

use crate::kernel::KERNEL;
use crate::kprintln;

pub static HEAP_ALLOCATOR: LockedHeap<27> = LockedHeap::<27>::new();

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

pub fn initialize_heap(regions: &[MemoryRegion]) {
    for region in regions {
        let end = region.start + region.size;
        kprintln!(
            "[MEMORY] Allocating region: {}B at 0x{:x}-0x{:x}",
            region.size, region.start, end
        );
        unsafe {
            HEAP_ALLOCATOR.lock().init(region.start, region.size);
        }
    }
    kprintln!("[MEMORY] Heap initialized successfully!");
}

const BLOCK_SIZE: usize = 64 * 1024;

pub static KERNEL_ALLOCATOR: LayeredAllocator = LayeredAllocator::new();
pub static USER_ALLOCATOR: LayeredAllocator = LayeredAllocator::new();

pub struct LayeredAllocator {
    heap: LockedHeap<27>,
}

impl LayeredAllocator {
    const fn new() -> Self {
        LayeredAllocator {
            heap: LockedHeap::new(),
        }
    }

    fn grow(&self) {
        let layout = unsafe { Layout::from_size_align_unchecked(BLOCK_SIZE, BLOCK_SIZE) };
        let ptr = unsafe { HEAP_ALLOCATOR.alloc(layout) };
        if !ptr.is_null() {
            unsafe {
                self.heap.lock().add_to_heap(ptr as usize, ptr as usize + BLOCK_SIZE);
            }
        }
    }
}

unsafe impl GlobalAlloc for LayeredAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        match self.heap.lock().alloc(layout) {
            Ok(ptr) => ptr.as_ptr(),
            Err(_) => {
                self.grow();
                self.heap
                    .lock()
                    .alloc(layout)
                    .map_or(null_mut(), |p| p.as_ptr())
            }
        }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        if let Some(non_null) = NonNull::new(ptr) {
            self.heap.lock().dealloc(non_null, layout);
        }
    }
}
