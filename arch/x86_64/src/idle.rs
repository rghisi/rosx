use alloc::boxed::Box;
use core::arch::asm;
use kernel::task::{FunctionTask, Task};

pub fn idle_task_factory() -> Box<Task> {
    FunctionTask::new("[K] Idle", idle_job)
}

fn idle_job() {
    loop {
        unsafe {
            asm!("hlt");
        }
    }
}
