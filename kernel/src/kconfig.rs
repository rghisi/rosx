use alloc::boxed::Box;
use crate::cpu::Cpu;
use crate::main_thread::MainThread;
use crate::mlfq_scheduler::MlfqScheduler;
use crate::scheduler::{Scheduler, SchedulerFactory};
use crate::task::SharedTask;

pub struct KConfig {
    pub cpu: &'static dyn Cpu,
    pub idle_task_factory: fn() -> SharedTask,
    pub scheduler_factory: SchedulerFactory,
}

unsafe impl Sync for KConfig {}

impl KConfig {
    pub fn mfq_scheduler() -> Box<dyn Scheduler> {
        Box::new(MlfqScheduler::new())
    }

    pub fn fifo_scheduler() -> Box<dyn Scheduler> {
        Box::new(MainThread::new())
    }
}
