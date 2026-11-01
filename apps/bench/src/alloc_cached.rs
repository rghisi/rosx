use crate::windows;
use alloc::alloc::{alloc, dealloc};
use alloc::format;
use alloc::string::String;
use core::alloc::Layout;
use core::hint::black_box;
use usrlib::println;

const ALIGNMENT: usize = 8;

const SIZES: &[usize] = &[
    16, 24, 32, 64, 128, 256, 512, 1000, 1024, 2048, 4096, 4097, 8192, 16384, 32768, 65536, 131072,
    262144, 524288, 1048576, 2097152, 4194304,
];

pub fn run() {
    println!(
        "bench.start suite=alloc_cached shape=pairs align={}",
        ALIGNMENT
    );
    for size in SIZES {
        let detail: String = format!("size={} align={}", size, ALIGNMENT);
        windows::run_phase("alloc_cached", &detail, &mut || allocate_and_release(*size));
    }
    println!("bench.done suite=alloc_cached");
}

fn allocate_and_release(size: usize) -> bool {
    let Ok(layout) = Layout::from_size_align(size, ALIGNMENT) else {
        return false;
    };
    let block = unsafe { alloc(layout) };
    if block.is_null() {
        return false;
    }
    black_box(block);
    unsafe { dealloc(block, layout) };
    true
}
