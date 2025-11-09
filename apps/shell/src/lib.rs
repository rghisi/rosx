#![cfg_attr(not(test), no_std)]

#[cfg(test)]
extern crate std as core;

extern crate usrlib;
extern crate system;
extern crate alloc;

pub mod shell;