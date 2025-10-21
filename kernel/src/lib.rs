#![cfg_attr(not(test), no_std)]


#[cfg(test)]
extern crate std as core;

extern crate alloc;

pub mod cpu;
pub mod runnable;
pub mod task;
pub mod function_task;
pub mod kernel;
pub mod task_queue;
pub mod task_fifo_queue;
pub mod task_scheduler_round_robin;
pub(crate) mod context_switcher;
pub mod debug;
pub(crate) mod state;
pub mod task_scheduler;
pub mod kconfig;
