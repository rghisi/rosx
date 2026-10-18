use crate::arch;
use crate::syscall::Syscall;
use alloc::string::String;
use alloc::vec::Vec;
use core::alloc::{GlobalAlloc, Layout};
use core::panic::PanicInfo;
use system::syscall_numbers::SyscallNum;

struct SyscallAllocator;

unsafe impl GlobalAlloc for SyscallAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        Syscall::alloc(layout.size(), layout.align())
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        Syscall::dealloc(ptr, layout.size(), layout.align());
    }
}

#[global_allocator]
static RUNTIME_ALLOCATOR: SyscallAllocator = SyscallAllocator;

pub fn args(args_ptr: usize, args_len: usize) -> Vec<String> {
    if args_len == 0 {
        return Vec::new();
    }
    let blob = unsafe { core::slice::from_raw_parts(args_ptr as *const u8, args_len) };
    system::args::decode(blob)
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
    arch::raw_syscall(SyscallNum::Print as usize, text.as_ptr() as usize, text.len(), 0);
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
