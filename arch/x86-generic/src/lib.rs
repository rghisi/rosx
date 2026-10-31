#![cfg_attr(not(test), no_std)]

pub mod ansi_parser;
#[cfg(not(test))]
pub mod debug_console;
#[cfg(not(test))]
pub mod port;
