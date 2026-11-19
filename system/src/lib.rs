#![cfg_attr(not(test), no_std)]

#[cfg(feature = "alloc")]
extern crate alloc;

pub mod args;
pub mod boot_disk;
pub mod boot_info;
#[cfg(feature = "alloc")]
pub mod boot_plan;
pub mod syscall_numbers;
pub mod ipc;
pub mod error;
pub mod exit;
pub mod irq;
pub mod kstats;
#[cfg(feature = "alloc")]
pub mod rom;
pub mod waitset;

