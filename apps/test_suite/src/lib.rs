#![cfg_attr(not(test), no_std)]

#[cfg(test)]
extern crate std as core;

extern crate alloc;
extern crate system;
extern crate usrlib;

pub mod app;
mod random;
mod allocation_test;
mod context_switching;
