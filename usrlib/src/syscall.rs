use core::fmt;
use system::syscall_numbers::SyscallNum;
use system::future::FutureHandle;
use crate::arch;

pub struct Syscall {}

impl Syscall {
    pub fn exec(entrypoint: usize) -> FutureHandle {
        let raw = arch::raw_syscall(SyscallNum::Exec as u64, entrypoint as u64, 0, 0);
        FutureHandle(raw as u64)
    }

    pub fn task_yield() {
        arch::raw_syscall(SyscallNum::Yield as u64, 0, 0, 0);
    }

    pub fn sleep(ms: u64) {
        arch::raw_syscall(SyscallNum::Sleep as u64, ms, 0, 0);
    }

    pub fn wait_future(handle: FutureHandle) {
        arch::raw_syscall(SyscallNum::WaitFuture as u64, handle.0, 0, 0);
    }

    pub fn is_future_completed(handle: FutureHandle) -> bool {
        let result = arch::raw_syscall(SyscallNum::IsFutureCompleted as u64, handle.0, 0, 0);
        result != 0
    }

    pub fn print(args: fmt::Arguments) {
        let s = alloc::fmt::format(args);
        arch::raw_syscall(SyscallNum::Print as u64, s.as_ptr() as u64, s.len() as u64, 0);
    }

    pub fn read_char() -> char {
        let c = arch::raw_syscall(SyscallNum::ReadChar as u64, 0, 0, 0);
        core::char::from_u32(c as u32).unwrap_or('\0')
    }
}

