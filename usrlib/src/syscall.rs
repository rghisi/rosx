use alloc::boxed::Box;
use core::fmt;
use system::syscall_numbers::SyscallNum;
use system::future::{FutureHandle, FutureResult};
use system::ipc::{IpcBindingError, IpcBindingHandle, IpcConnectionError, IpcSendError, IpcConnectionHandle};
use crate::arch;

pub struct Syscall {}

impl Syscall {
    pub fn exec(entrypoint: usize) -> FutureHandle {
        let raw = arch::raw_syscall(SyscallNum::Exec as usize, entrypoint as usize, 0, 0);
        FutureHandle::unpack(raw)
    }

    pub fn load(elf: &'static [u8]) -> FutureHandle {
        let elf_ptr = Box::into_raw(Box::new(elf)) as usize;
        let raw = arch::raw_syscall(SyscallNum::LoadElf as usize, elf_ptr as usize, 0, 0);
        FutureHandle::unpack(raw)
    }

    pub fn task_yield() {
        arch::raw_syscall(SyscallNum::Yield as usize, 0, 0, 0);
    }

    pub fn sleep(ms: u64) {
        arch::raw_syscall(SyscallNum::Sleep as usize, ms as usize, 0, 0);
    }

    pub fn wait_future(handle: FutureHandle) -> FutureResult {
        let result = arch::raw_syscall(SyscallNum::WaitFuture as usize, handle.pack(), 0, 0);
        unsafe { *Box::from_raw(result as *mut FutureResult) }
    }

    pub fn is_future_completed(handle: FutureHandle) -> bool {
        let result = arch::raw_syscall(SyscallNum::IsFutureCompleted as usize, handle.pack(), 0, 0);
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

    pub fn ipc_connect(service: &str) -> Result<IpcConnectionHandle, IpcConnectionError> {
        let boxed = Box::into_raw(Box::new(service)) as usize;
        let result = arch::raw_syscall(SyscallNum::IpcConnect as usize, boxed, 0, 0);
        unsafe { *Box::from_raw(result as *mut Result<IpcConnectionHandle, IpcConnectionError>) }
    }

    pub fn ipc_disconnect(connection_handle: IpcConnectionHandle) {
        arch::raw_syscall(SyscallNum::IpcDisconnect as usize, connection_handle.index as usize, connection_handle.generation as usize, 0usize);
    }

    pub fn ipc_send(connection_handle: IpcConnectionHandle, value: usize) -> Result<(), IpcSendError> {
        let result_pointer = arch::raw_syscall(SyscallNum::IpcSend as usize, connection_handle.index as usize, connection_handle.generation as usize, value);
        unsafe { *Box::from_raw(result_pointer as *mut Result<(), IpcSendError>) }
    }

    pub fn ipc_receive(connection_handle: IpcConnectionHandle) -> FutureHandle {
        let raw = arch::raw_syscall(SyscallNum::IpcReceive as usize, connection_handle.index as usize, connection_handle.generation as usize, 0usize);
        FutureHandle::unpack(raw)
    }

    pub fn ipc_bind(service: &str) -> Result<IpcBindingHandle, IpcBindingError> {
        let boxed = Box::into_raw(Box::new(service)) as usize;
        let result = arch::raw_syscall(SyscallNum::IpcBind as usize, boxed, 0, 0);
        unsafe { *Box::from_raw(result as *mut Result<IpcBindingHandle, IpcBindingError>) }
    }

    pub fn ipc_receive_from_client(binding_handle: IpcBindingHandle) -> FutureHandle {
        let raw = arch::raw_syscall(SyscallNum::IpcReceiveFromClient as usize, binding_handle.index as usize, binding_handle.generation as usize, 0usize);
        FutureHandle::unpack(raw)
    }

    pub fn ipc_send_to_client(connection_handle: IpcConnectionHandle, value: usize) -> Result<(), IpcSendError> {
        let result_pointer = arch::raw_syscall(SyscallNum::IpcSendToClient as usize, connection_handle.index as usize, connection_handle.generation as usize, value);
        unsafe { *Box::from_raw(result_pointer as *mut Result<(), IpcSendError>) }
    }
}

