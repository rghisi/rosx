#![no_std]

extern crate alloc;

use usrlib::println;
use usrlib::syscall::Syscall;

mod allocation_test;
mod context_switching;
mod random;

fn main() {
    println!("=== RosX Test Suite Started ===");

    if let Some(fh) = Syscall::exec(allocation_test::run as *const () as usize) {
        Syscall::wait_future(fh);
    }
    if let Some(fh) = Syscall::exec(context_switching::worker_context_switch as *const () as usize)
    {
        Syscall::wait_future(fh);
    }
    if let Some(fh) = Syscall::exec(worker_mixed_load as *const () as usize) {
        Syscall::wait_future(fh);
    }

    println!("=== Main Thread Finished ===");
}

fn worker_mixed_load() {
    println!("[MixWorker] Starting Mixed Load Test...");

    if let Some(mem) = Syscall::exec(allocation_test::run as *const () as usize) {
        if let Some(ctx) =
            Syscall::exec(context_switching::worker_context_switch as *const () as usize)
        {
            Syscall::wait_future(mem);
            Syscall::wait_future(ctx);
        }
    }

    println!("[MixWorker] Finished Mixed Load Test...");
}
