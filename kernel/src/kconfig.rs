use alloc::boxed::Box;
use cpu::Cpu;
use task_scheduler::TaskScheduler;
use task::{SharedTask, Task};

pub struct KConfig {
    pub cpu: &'static dyn Cpu,
    pub scheduler: fn() -> Box<dyn TaskScheduler>,
    pub idle_task: fn() -> Box<Task>,
}

unsafe impl Sync for KConfig {

}