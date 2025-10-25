use alloc::boxed::Box;
use cpu::Cpu;
use task::{SharedTask, Task};
use task_queue::TaskQueue;

pub struct KConfig {
    pub cpu: &'static dyn Cpu,
    pub user_thread_queue: fn() -> Box<dyn TaskQueue>,
    pub idle_task: fn() -> Box<Task>,
}

unsafe impl Sync for KConfig {

}