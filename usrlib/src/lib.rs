#![cfg_attr(not(test), no_std)]

#[cfg(test)]
extern crate std as core;

extern crate kernel;
extern crate alloc;
extern crate system;

pub mod out;

pub mod syscall;