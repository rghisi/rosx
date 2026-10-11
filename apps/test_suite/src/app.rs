use crate::{allocation_test, context_switching};
use usrlib::println;
use usrlib::syscall::Syscall;

pub fn main() {
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

pub fn worker_mixed_load() {
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
