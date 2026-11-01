use crate::windows;
use alloc::format;
use alloc::string::String;
use core::hint::black_box;
use usrlib::println;
use usrlib::syscall::Syscall;

const ALIGNMENT: usize = 8;

const SIZES: &[usize] = &[
    16, 24, 32, 64, 128, 256, 512, 1000, 1024, 2048, 4096, 4097, 8192, 16384, 32768, 65536, 131072,
    262144, 524288, 1048576, 2097152, 4194304,
];

pub fn run() {
    println!("bench.start suite=alloc shape=pairs align={}", ALIGNMENT);
    for size in SIZES {
        let detail: String = format!("size={} align={}", size, ALIGNMENT);
        windows::run_phase("alloc", &detail, &mut || allocate_and_release(*size));
    }
    println!("bench.done suite=alloc");
}

fn allocate_and_release(size: usize) -> bool {
    let block = Syscall::alloc(size, ALIGNMENT);
    if block.is_null() {
        return false;
    }
    black_box(block);
    Syscall::dealloc(block, size, ALIGNMENT);
    true
}
