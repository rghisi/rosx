use core::fmt;
use system::syscall_numbers::SyscallNum;
use crate::arch;

pub struct Syscall {}

impl Syscall {
    pub fn exec(entrypoint: usize) {
        arch::raw_syscall(SyscallNum::Exec as u64, entrypoint as u64, 0, 0);
    }

    pub fn task_yield() {
        arch::raw_syscall(SyscallNum::Yield as u64, 0, 0, 0);
    }

    pub fn sleep(ms: u64) {
        arch::raw_syscall(SyscallNum::Sleep as u64, ms, 0, 0);
    }

    pub fn print(args: fmt::Arguments) {
        let s = alloc::fmt::format(args);
        arch::raw_syscall(SyscallNum::Print as u64, s.as_ptr() as u64, s.len() as u64, 0);
    }
}
