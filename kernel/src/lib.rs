#![cfg_attr(not(test), no_std)]


#[cfg(test)]
extern crate std as core;

extern crate alloc;
extern crate system;
extern crate lazy_static;
extern crate spin;

pub mod cpu;
pub mod task;
pub mod function_task;
pub mod kernel;
pub mod task_queue;
pub mod task_fifo_queue;
pub mod main_thread;
pub(crate) mod context_switcher;
pub mod default_output;
pub(crate) mod state;
pub mod kconfig;
pub mod messages;
mod keyboard;
mod circular_queue;
mod growing_circular_queue;
mod messaging;
pub mod pipe;
mod task_arena;
pub(crate) mod task_manager;
pub mod allocator;
mod panic;
pub mod syscall;
pub mod future;
pub mod file_manager;
pub mod file_arena;
