use alloc::boxed::Box;
use core::arch::asm;
use kernel::function_task::FunctionTask;
use kernel::task::Task;
use usrlib::println;

pub fn idle_task_factory() -> Box<Task> {
    FunctionTask::new("[K] Idle", idle_job)
}

fn idle_job() {
    println!("Idle Task Start");
    let mut counter = 0;
    loop {
        if counter % 100 == 0 {
            println!("Idling... {}", counter);
        }
        counter += 1;
        unsafe { asm!("hlt"); }
    }
}