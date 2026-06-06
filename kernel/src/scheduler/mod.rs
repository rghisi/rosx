pub mod algorithm;
pub mod fifo_strategy;
pub mod mlfq_strategy;
mod scheduler_engine;
mod timer;

use alloc::boxed::Box;
use system::future::FutureHandle;
use crate::messages::HardwareInterrupt;
use crate::task::TaskHandle;

pub use algorithm::SchedulingAlgorithm;
pub use scheduler_engine::SchedulerEngine;

pub trait Scheduler {
    fn run(&mut self);
    fn push_task(&mut self, handle: TaskHandle);
    fn push_blocked(&mut self, task_handle: TaskHandle, future_handle: FutureHandle);
    fn push_hardware_interrupt(&mut self, interrupt: HardwareInterrupt);
    fn set_idle_task(&mut self, handle: TaskHandle) -> Result<(), ()>;
    fn should_preempt(&mut self) -> bool;
}

pub type SchedulerFactory = fn() -> Box<SchedulerEngine>;

pub fn mfq_scheduler() -> Box<SchedulerEngine> {
    Box::new(SchedulerEngine::new(mlfq_strategy::MlfqStrategy::new()))
}

pub fn fifo_scheduler() -> Box<SchedulerEngine> {
    Box::new(SchedulerEngine::new(fifo_strategy::FifoStrategy::new()))
}