use crate::arch;
use crate::syscall::Syscall;
use alloc::string::String;
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

#[lang = "start"]
fn lang_start<T: 'static>(
    main: fn() -> T,
    _argc: isize,
    _argv: *const *const u8,
    _sigpipe: u8,
) -> isize {
    main();
    0
}

#[macro_export]
macro_rules! app_main {
    (fn main($args:ident : Vec < String >) $body:block) => {
        fn rosx_app_entry($args: ::alloc::vec::Vec<String>) $body

        #[unsafe(no_mangle)]
        pub extern "C" fn main(args_ptr: usize, args_len: usize) -> isize {
            $crate::runtime::start(rosx_app_entry, args_ptr, args_len)
        }
    };
}

pub fn start(entry: fn(alloc::vec::Vec<String>), args_ptr: usize, args_len: usize) -> isize {
    let args = if args_len == 0 {
        alloc::vec::Vec::new()
    } else {
        let blob = unsafe { core::slice::from_raw_parts(args_ptr as *const u8, args_len) };
        system::args::decode(blob)
    };
    entry(args);
    0
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
