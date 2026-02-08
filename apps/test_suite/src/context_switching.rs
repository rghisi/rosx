use usrlib::syscall::Syscall;
use usrlib::println;

const CONTEXT_SWITCH_ITERATIONS: usize = 1000000;
pub fn worker_context_switch() {
    println!("[CtxWorker] Starting Context Switch Test...");
    for i in 0..CONTEXT_SWITCH_ITERATIONS {
        if i % 100000 == 0 {
            println!("[CtxWorker] Progress: {}/{}", i, CONTEXT_SWITCH_ITERATIONS);
        }
        // Heavy yielding to force context switches
        Syscall::task_yield();
    }
    println!("\n[CtxWorker] [PASS] Context Switch Test Completed");
}