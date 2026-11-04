#![no_std]

extern crate alloc;

use system::stack::STACK_GUARD_MAGIC;
use usrlib::println;
use usrlib::syscall::Syscall;

const SEARCH_BYTES: usize = 256 * 1024;

fn locate_guard_below(start: usize) -> Option<usize> {
    let word_bytes = core::mem::size_of::<usize>();
    let mut address = start & !(word_bytes - 1);
    let floor = address - SEARCH_BYTES;
    while address > floor {
        let candidate = unsafe { core::ptr::read_volatile(address as *const usize) };
        if candidate == STACK_GUARD_MAGIC {
            return Some(address);
        }
        address -= word_bytes;
    }
    None
}

fn main() {
    let probe = 0usize;
    let stack_address = &probe as *const usize as usize;
    println!("BOOM hunting guard below 0x{:x}", stack_address);
    match locate_guard_below(stack_address) {
        Some(address) => {
            println!("BOOM found guard at 0x{:x}", address);
            unsafe {
                core::ptr::write_volatile(address as *mut usize, STACK_GUARD_MAGIC ^ 0xFFFF_FFFF);
            }
            println!("BOOM corrupted guard, sleeping while the kernel notices");
        }
        None => println!("BOOM guard not found within search range"),
    }
    loop {
        Syscall::sleep(50);
    }
}
