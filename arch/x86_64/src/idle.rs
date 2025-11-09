use alloc::boxed::Box;
use core::arch::asm;
use kernel::function_task::FunctionTask;
use kernel::syscall::get_system_time;
use kernel::task::Task;
use usrlib::println;

pub fn idle_task_factory() -> Box<Task> {
    FunctionTask::new("[K] Idle", idle_job)
}

fn idle_job() {
    // println!("\nIdle Task Started {}", get_system_time());
    let mut counter = 0;
    loop {
        if counter % 100 == 0 {
            // println!("Idling... {}", get_system_time());
        }
        counter += 1;
        unsafe {
            asm!("hlt");
        }
    }
}
