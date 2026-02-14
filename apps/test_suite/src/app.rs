use crate::{allocation_test, context_switching};
use usrlib::println;
use usrlib::syscall::Syscall;

pub fn main() {
    println!("=== RosX Test Suite Started ===");

    Syscall::wait_future(Syscall::exec(allocation_test::run as usize));
    Syscall::wait_future(Syscall::exec(context_switching::worker_context_switch as usize));
    Syscall::wait_future(Syscall::exec(worker_mixed_load as usize));

    println!("=== Main Thread Finished ===");
}

pub fn worker_mixed_load() {
    println!("[MixWorker] Starting Mixed Load Test...");

    let mem = Syscall::exec(allocation_test::run as usize);
    let ctx = Syscall::exec(context_switching::worker_context_switch as usize);

    Syscall::wait_future(mem);
    Syscall::wait_future(ctx);

    println!("[MixWorker] Finished Mixed Load Test...");
}
