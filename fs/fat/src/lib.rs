#![cfg_attr(not(test), no_std)]

extern crate alloc;

pub mod boot;
pub mod device;
pub mod dir;
pub mod fat;
pub mod file;
pub mod path;
pub mod volume;

pub use volume::{FileSystem, FatHandle, FatMetadata};
