#![cfg_attr(not(test), no_std)]

#[cfg(test)]
extern crate std as core;

extern crate alloc;
extern crate system;
extern crate usrlib;

mod allocation_test;
pub mod app;
mod context_switching;
mod random;
