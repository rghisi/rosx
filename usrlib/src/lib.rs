#![cfg_attr(not(test), no_std)]

extern crate kernel;
extern crate alloc;
extern crate system;

pub mod out;

pub mod syscall;