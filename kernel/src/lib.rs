#![cfg_attr(not(test), no_std)]
#![cfg_attr(not(test), feature(alloc_error_handler))]

extern crate alloc;
extern crate system;

pub mod collections;
pub mod elf;
pub mod future;
pub mod handle;
pub mod ipc;
pub mod kconfig;
pub mod kernel;
pub(crate) mod kernel_cell;
pub(crate) mod kernel_services;
pub mod kstatsd;
pub mod memory;
pub mod once;
pub mod panic;
pub mod platform;
pub mod ports;
pub mod rom;
pub mod scheduler;
pub(crate) mod state;
pub(crate) mod wake_probe;
pub mod syscall;
pub mod task;
pub(crate) mod task_manager;
pub mod task_stack;
