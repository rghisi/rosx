use crate::arch;
use alloc::vec::Vec;
#[cfg(not(feature = "slab-allocator"))]
use core::alloc::{GlobalAlloc, Layout};
use core::ffi::CStr;
use core::panic::PanicInfo;
use core::sync::atomic::{AtomicI32, AtomicPtr, Ordering};
use system::syscall_numbers::SyscallNum;

#[cfg(feature = "slab-allocator")]
use crate::allocator::UsrAllocator;

#[cfg(feature = "slab-allocator")]
#[global_allocator]
static RUNTIME_ALLOCATOR: UsrAllocator = UsrAllocator;

#[cfg(not(feature = "slab-allocator"))]
struct SyscallAllocator;

#[cfg(not(feature = "slab-allocator"))]
unsafe impl GlobalAlloc for SyscallAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        crate::syscall::Syscall::alloc(layout.size(), layout.align())
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        crate::syscall::Syscall::dealloc(ptr, layout.size(), layout.align());
    }
}

#[cfg(not(feature = "slab-allocator"))]
#[global_allocator]
static RUNTIME_ALLOCATOR: SyscallAllocator = SyscallAllocator;

static ARGUMENT_COUNT: AtomicI32 = AtomicI32::new(0);
static ARGUMENT_VECTORS: AtomicPtr<u8> = AtomicPtr::new(core::ptr::null_mut());

#[lang = "start"]
fn start<T: 'static>(main: fn() -> T, argc: isize, argv: *const *const u8, _sigpipe: u8) -> isize {
    ARGUMENT_COUNT.store(argc as i32, Ordering::Relaxed);
    ARGUMENT_VECTORS.store(argv as *mut u8, Ordering::Relaxed);
    main();
    0
}

pub fn argc() -> i32 {
    ARGUMENT_COUNT.load(Ordering::Relaxed)
}

pub fn argv() -> *const *const u8 {
    ARGUMENT_VECTORS.load(Ordering::Relaxed) as *const *const u8
}

pub fn args() -> Vec<&'static str> {
    let argv = argv();
    if argv.is_null() {
        return Vec::new();
    }
    (0..argc())
        .filter_map(|index| {
            unsafe { CStr::from_ptr(argv.add(index as usize).read() as *const i8) }
                .to_str()
                .ok()
        })
        .collect()
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    match info.location() {
        Some(location) => {
            print_raw("panic at ");
            print_raw(location.file());
            print_raw(":");
            print_number(location.line());
        }
        None => print_raw("panic"),
    }
    print_raw("\n");
    loop {}
}

fn print_raw(text: &str) {
    arch::raw_syscall(
        SyscallNum::Print as usize,
        text.as_ptr() as usize,
        text.len(),
        0,
    );
}

fn print_number(value: u32) {
    let mut buffer = [0u8; 10];
    let mut index = buffer.len();
    let mut remaining = value;
    loop {
        index -= 1;
        buffer[index] = b'0' + (remaining % 10) as u8;
        remaining /= 10;
        if remaining == 0 {
            break;
        }
    }
    print_raw(core::str::from_utf8(&buffer[index..]).unwrap_or(""));
}
