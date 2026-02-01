#![cfg_attr(not(test), no_std)]

extern crate alloc;
extern crate kernel;
extern crate system;

pub mod out;
pub mod arch;
pub mod syscall;
