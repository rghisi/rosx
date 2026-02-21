pub mod fifo_scheduler;
pub mod mlfq_scheduler;
use alloc::boxed::Box;
use system::future::FutureHandle;
use crate::messages::HardwareInterrupt;
use crate::scheduler::fifo_scheduler::FifoScheduler;
use crate::scheduler::mlfq_scheduler::MlfqScheduler;
use crate::task::TaskHandle;

pub trait Scheduler {
    fn run(&mut self);
    fn push_task(&mut self, handle: TaskHandle);
    fn push_blocked(&mut self, task_handle: TaskHandle, future_handle: FutureHandle);
    fn push_hardware_interrupt(&mut self, interrupt: HardwareInterrupt);
    fn set_idle_task(&mut self, handle: TaskHandle) -> Result<(), ()>;
    fn should_preempt(&mut self) -> bool;
}

pub type SchedulerFactory = fn() -> Box<dyn Scheduler>;

pub fn mfq_scheduler() -> Box<dyn Scheduler> {
    Box::new(MlfqScheduler::new())
}

pub fn fifo_scheduler() -> Box<dyn Scheduler> {
    Box::new(FifoScheduler::new())
}