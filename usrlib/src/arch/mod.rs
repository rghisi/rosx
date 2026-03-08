#[cfg(target_arch = "x86_64")]
#[path = "x86_64.rs"]
mod implementation;

#[cfg(target_arch = "x86")]
#[path = "x86_32.rs"]
mod implementation;

#[cfg(target_arch = "m68k")]
#[path = "m68k.rs"]
mod implementation;

pub use implementation::raw_syscall;
