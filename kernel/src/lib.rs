#![cfg_attr(not(test), no_std)]

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
pub mod generational_arena;
pub(crate) mod task_manager;
pub mod allocator;
pub mod panic;
pub mod syscall;
pub mod future;
