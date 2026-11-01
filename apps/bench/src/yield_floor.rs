use crate::windows;
use usrlib::println;
use usrlib::syscall::Syscall;

pub fn run() {
    println!("bench.start suite=yield shape=voluntary");
    windows::run_phase("yield", "shape=voluntary", &mut yield_once);
    println!("bench.done suite=yield");
}

fn yield_once() -> bool {
    Syscall::task_yield();
    true
}
