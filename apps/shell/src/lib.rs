#![cfg_attr(not(test), no_std)]

#[cfg(test)]
extern crate std as core;

extern crate alloc;

pub mod command;
pub mod launcher;
pub mod shell;
