use alloc::boxed::Box;
use alloc::string::String;
use core::fmt;
use system::syscall_numbers::SyscallNum;
use system::future::FutureHandle;
use system::future::Future;
use system::ipc::{IpcError, IpcReplyFuture, IpcServerHandle};
use crate::arch;

pub struct Syscall {}

impl Syscall {
    pub fn exec(entrypoint: usize) -> FutureHandle {
        let raw = arch::raw_syscall(SyscallNum::Exec as usize, entrypoint as usize, 0, 0);
        FutureHandle {
            index: (raw >> 32) as u32,
            generation: raw as u32,
        }
    }

    pub fn load(elf: &'static [u8]) -> FutureHandle {
        let elf_ptr = Box::into_raw(Box::new(elf)) as usize;
        let raw = arch::raw_syscall(SyscallNum::LoadElf as usize, elf_ptr as usize, 0, 0);
        FutureHandle {
            index: (raw >> 32) as u32,
            generation: raw as u32,
        }
    }

    pub fn task_yield() {
        arch::raw_syscall(SyscallNum::Yield as usize, 0, 0, 0);
    }

    pub fn sleep(ms: u64) {
        arch::raw_syscall(SyscallNum::Sleep as u64, ms, 0, 0);
    }

    pub fn wait_future(handle: FutureHandle) -> Box<dyn Future + Send + Sync> {
        let packed = (handle.index as u64) << 32 | (handle.generation as u64);
        let result = arch::raw_syscall(SyscallNum::WaitFuture as usize, packed, 0, 0);
        let r: Box<dyn Future + Send + Sync> = unsafe { *Box::from_raw(result as *mut Box<dyn Future + Send + Sync>) };
        r
    }

    pub fn is_future_completed(handle: FutureHandle) -> bool {
        let packed = (handle.index as u64) << 32 | (handle.generation as u64);
        let result = arch::raw_syscall(SyscallNum::IsFutureCompleted as usize, packed, 0, 0);
        result != 0
    }

    pub fn print(args: fmt::Arguments) {
        let s = alloc::fmt::format(args);
        arch::raw_syscall(SyscallNum::Print as usize, s.as_ptr() as usize, s.len(), 0);
    }

    pub fn read_char() -> char {
        let c = arch::raw_syscall(SyscallNum::ReadChar as usize, 0, 0, 0);
        core::char::from_u32(c as u32).unwrap_or('\0')
    }

    pub fn try_read_char() -> Option<char> {
        let c = arch::raw_syscall(SyscallNum::TryReadChar as usize, 0, 0, 0);
        if c == 0 {
            None
        } else {
            core::char::from_u32(c as u32)
        }
    }

    pub fn alloc(size: usize, align: usize) -> *mut u8 {
        arch::raw_syscall(SyscallNum::Alloc as usize, size, align, 0) as *mut u8
    }

    pub fn dealloc(ptr: *mut u8, size: usize, align: usize) {
        arch::raw_syscall(SyscallNum::Dealloc as usize, ptr as usize, size, align);
    }

    pub fn ipc_find(service: &str) -> Result<IpcServerHandle, IpcError> {
        let boxed = Box::into_raw(Box::new(service)) as usize;
        let result = arch::raw_syscall(SyscallNum::IpcFind as usize, boxed, 0, 0);
        unsafe { *Box::from_raw(result as *mut Result<IpcServerHandle, IpcError>) }
    }

    pub fn ipc_send(handle: IpcServerHandle, value: u32) -> IpcReplyFuture {
        let result = arch::raw_syscall(SyscallNum::IpcSend as usize, handle.index as usize, handle.generation as usize, value as usize);
        unsafe { *Box::from_raw(result as *mut IpcReplyFuture) }
    }
}

