#![cfg_attr(not(test), no_std)]
#![cfg_attr(not(test), feature(alloc_error_handler))]

extern crate alloc;
extern crate collections;
extern crate lazy_static;
extern crate system;

pub mod cpu;
pub mod default_output;
pub mod elf;
pub mod future;
pub mod ipc;
pub mod kconfig;
pub mod kernel;
pub(crate) mod kernel_cell;
pub(crate) mod kernel_services;
mod keyboard;
pub mod memory;
pub mod messages;
pub mod once;
pub mod panic;
pub mod scheduler;
pub(crate) mod state;
pub mod syscall;
pub mod task;
pub(crate) mod task_manager;
