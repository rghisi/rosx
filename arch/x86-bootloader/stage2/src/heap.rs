use core::alloc::GlobalAlloc;
use core::alloc::Layout;
use core::sync::atomic::AtomicUsize;
use core::sync::atomic::Ordering;
use rosx_boot_common::biosdisk::HEAP_ADDRESS;
use rosx_boot_common::biosdisk::HEAP_BYTES;
use rosx_boot_common::serial::Serial;
use rosx_boot_common::DEBUG_EXIT_FAILURE;
use rosx_boot_common::debug_exit;

pub struct BumpAllocator {
    next: AtomicUsize,
    end: usize,
}

impl BumpAllocator {
    pub const fn new() -> Self {
        Self {
            next: AtomicUsize::new(HEAP_ADDRESS),
            end: HEAP_ADDRESS + HEAP_BYTES,
        }
    }
}

unsafe impl GlobalAlloc for BumpAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let align = layout.align().max(core::mem::size_of::<usize>());
        let mask = align - 1;
        let current = self.next.load(Ordering::Relaxed);
        let begin = (current + mask) & !mask;
        let Some(finish) = begin.checked_add(layout.size()) else {
            refuse();
        };
        if finish > self.end {
            refuse();
        }
        self.next.store(finish, Ordering::Relaxed);
        begin as *mut u8
    }

    unsafe fn dealloc(&self, _ptr: *mut u8, _layout: Layout) {}
}

fn refuse() -> ! {
    Serial::COM1.write_line("[ERR] heap");
    debug_exit(DEBUG_EXIT_FAILURE)
}

#[global_allocator]
static HEAP: BumpAllocator = BumpAllocator::new();
