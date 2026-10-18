#![cfg_attr(all(not(test), feature = "runtime"), feature(lang_items))]
#![cfg_attr(all(not(test), feature = "runtime"), allow(internal_features))]
#![cfg_attr(not(test), no_std)]

extern crate alloc;
extern crate system;

#[cfg(all(feature = "runtime", not(test)))]
pub mod runtime;
pub mod out;
pub mod arch;
pub mod error;
pub mod handle;
pub mod ipc;
pub mod syscall;
