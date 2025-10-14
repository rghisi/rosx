#![cfg_attr(not(test), no_std)]


#[cfg(test)]
extern crate std as core;

extern crate alloc;

pub mod cpu;
pub mod runnable;
pub mod task;
pub mod function_task;
pub mod kernel;
pub mod scheduler;
pub mod simple_scheduler;
pub(crate) mod main_thread;
pub(crate) mod context_switcher;
pub mod debug;
pub(crate) mod state;
pub(crate) mod wrappers;
