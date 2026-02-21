#![cfg_attr(not(test), no_std)]

#[cfg(test)]
extern crate std as core;

extern crate alloc;
extern crate lazy_static;
extern crate system;
extern crate usrlib;

pub mod shell;
