#![no_std]

extern crate alloc;

use alloc::string::String;
use usrlib::{print, println};
use usrlib::syscall::Syscall;

const TICK_MS: u64 = 100;
const ANNOUNCE_EVERY_TICKS: u32 = 10;

fn main() {
    println!("spinner ready");
    let mut ticks: u32 = 0;
    loop {
        Syscall::sleep(TICK_MS);
        ticks += 1;
        if ticks % ANNOUNCE_EVERY_TICKS == 0 {
            let line: String = alloc::format!("spinner tick {}\n", ticks);
            print!("{}", line);
        }
    }
}
