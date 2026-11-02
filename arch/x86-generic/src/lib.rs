#![cfg_attr(not(test), no_std)]

pub mod ansi_parser;
#[cfg(not(test))]
pub mod debug_console;
pub mod multiboot;
#[cfg(not(test))]
pub mod port;
#[cfg(not(test))]
pub mod vga_buffer;
