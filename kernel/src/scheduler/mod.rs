pub mod algorithm;
pub mod fifo_strategy;
pub mod mlfq_strategy;
mod scheduler_engine;
mod timer;

use alloc::boxed::Box;

pub use algorithm::SchedulingAlgorithm;
pub use scheduler_engine::SchedulerEngine;

pub type SchedulerFactory = fn() -> Box<SchedulerEngine>;

pub fn mfq_scheduler() -> Box<SchedulerEngine> {
    Box::new(SchedulerEngine::new(mlfq_strategy::MlfqStrategy::new()))
}

pub fn fifo_scheduler() -> Box<SchedulerEngine> {
    Box::new(SchedulerEngine::new(fifo_strategy::FifoStrategy::new()))
}