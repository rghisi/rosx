#[cfg(target_arch = "x86_64")]
#[path = "x86_64.rs"]
mod implementation;

pub use implementation::raw_syscall;
