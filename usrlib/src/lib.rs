#![cfg_attr(not(test), no_std)]

extern crate alloc;
extern crate system;

pub mod arch;
pub mod out;
pub mod syscall;
