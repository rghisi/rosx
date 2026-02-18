#![cfg_attr(not(test), no_std)]
#![cfg_attr(not(test), feature(alloc_error_handler))]

extern crate alloc;
extern crate collections;
extern crate lazy_static;
extern crate system;

pub mod allocator;
pub mod bitmap_chunk_allocator;
pub mod chunk_tracker;
pub mod free_list_allocator;
pub(crate) mod context_switcher;
pub mod cpu;
pub mod default_output;
pub mod elf;
pub mod function_task;
pub mod future;
pub mod kconfig;
pub mod kernel;
pub(crate) mod kernel_cell;
pub(crate) mod kernel_services;
mod keyboard;
pub mod main_thread;
pub mod messages;
pub mod once;
mod messaging;
pub mod panic;
pub mod pipe;
pub(crate) mod state;
pub mod syscall;
pub mod task;
pub mod task_fifo_queue;
pub(crate) mod task_manager;
pub mod task_queue;
