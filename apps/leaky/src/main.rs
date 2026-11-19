#![no_std]

use usrlib::println;
use usrlib::syscall::Syscall;

const LEAK_LIMIT: usize = 4096;

fn main() {
    let mut abandoned = 0usize;
    while abandoned < LEAK_LIMIT {
        match Syscall::waitset_create() {
            Ok(_) => abandoned += 1,
            Err(_) => break,
        }
    }
    println!("LEAKY ABANDONED {} WAITSETS", abandoned);
}
