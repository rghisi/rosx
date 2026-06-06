pub mod algorithm;
pub mod fifo_strategy;
pub mod mlfq_strategy;
mod scheduler;
mod timer;

use alloc::boxed::Box;

pub use algorithm::SchedulingAlgorithm;
pub use scheduler::Scheduler;

pub type SchedulerFactory = fn() -> Box<Scheduler>;

pub fn mfq_scheduler() -> Box<Scheduler> {
    Box::new(Scheduler::new(mlfq_strategy::MlfqStrategy::new()))
}

pub fn fifo_scheduler() -> Box<Scheduler> {
    Box::new(Scheduler::new(fifo_strategy::FifoStrategy::new()))
}