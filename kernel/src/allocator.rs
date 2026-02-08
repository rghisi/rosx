use core::alloc::{GlobalAlloc, Layout};
use core::mem::MaybeUninit;
use core::sync::atomic::{AtomicUsize, Ordering};
use crate::cpu::Cpu;

#[cfg_attr(not(test), global_allocator)]
pub static MEMORY_ALLOCATOR: MemoryAllocator = MemoryAllocator::new();

trait Xpto {}
pub struct MemoryAllocator {
    root: MaybeUninit<&'static (dyn GlobalAlloc + Sync)>,
    cpu: MaybeUninit<&'static dyn Cpu>,
    used: AtomicUsize,
}

impl MemoryAllocator {
    const fn new() -> Self {
        MemoryAllocator {
            root: MaybeUninit::uninit(),
            cpu: MaybeUninit::uninit(),
            used: AtomicUsize::new(0),
        }
    }

    pub unsafe fn init(&self, allocator: &'static (dyn GlobalAlloc + Sync), cpu: &'static dyn Cpu) {
        unsafe {
            core::ptr::addr_of!(self.root)
                .cast_mut()
                .write(MaybeUninit::new(allocator));
            core::ptr::addr_of!(self.cpu)
                .cast_mut()
                .write(MaybeUninit::new(cpu));
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
            let cpu = self.cpu.assume_init();
            let interrupts_enabled = cpu.are_interrupts_enabled();
            if interrupts_enabled {
                cpu.disable_interrupts();
            }

            self.used.fetch_add(layout.size(), Ordering::Relaxed);
            let ptr = self.root.assume_init().alloc(layout);

            if interrupts_enabled {
                cpu.enable_interrupts();
            }
            ptr
        }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe {
            let cpu = self.cpu.assume_init();
            let interrupts_enabled = cpu.are_interrupts_enabled();
            if interrupts_enabled {
                cpu.disable_interrupts();
            }

            self.used.fetch_sub(layout.size(), Ordering::Relaxed);
            self.root.assume_init().dealloc(ptr, layout);

            if interrupts_enabled {
                cpu.enable_interrupts();
            }
        }
    }
}
