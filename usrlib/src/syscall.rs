use alloc::boxed::Box;
use core::fmt;
use system::syscall_numbers::SyscallNum;
use system::future::{FutureHandle, FutureResult};
use system::ipc::{IpcBindingError, IpcBindingHandle, IpcConnectionError, IpcSendError, IpcConnectionHandle, IpcMessageError, IpcMessageHandle};
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

    pub fn ipc_bind(service: &str) -> Result<IpcBindingHandle, IpcBindingError> {
        let boxed = Box::into_raw(Box::new(service)) as usize;
        let result = arch::raw_syscall(SyscallNum::IpcBind as usize, boxed, 0, 0);
        unsafe { *Box::from_raw(result as *mut Result<IpcBindingHandle, IpcBindingError>) }
    }

    pub fn ipc_create_message(connection_handle: IpcConnectionHandle, data_size: usize) -> Result<IpcMessageHandle, IpcSendError> {
        let result_pointer = arch::raw_syscall(SyscallNum::IpcCreateMessage as usize, connection_handle.pack(), data_size, 0);
        unsafe { *Box::from_raw(result_pointer as *mut Result<IpcMessageHandle, IpcSendError>) }
    }

    pub fn ipc_write_message(message_handle: IpcMessageHandle, bytes: &[u8]) -> Result<(), IpcMessageError> {
        let result_pointer = arch::raw_syscall(SyscallNum::IpcWriteMessage as usize, message_handle.pack(), bytes.as_ptr() as usize, bytes.len());
        unsafe { *Box::from_raw(result_pointer as *mut Result<(), IpcMessageError>) }
    }

    pub fn ipc_send_message(message_handle: IpcMessageHandle) -> Result<(), IpcSendError> {
        let result_pointer = arch::raw_syscall(SyscallNum::IpcSendMessage as usize, message_handle.pack(), 0, 0);
        unsafe { *Box::from_raw(result_pointer as *mut Result<(), IpcSendError>) }
    }

    pub fn ipc_read_message(message_handle: IpcMessageHandle, dst: &mut [u8]) -> Result<usize, IpcMessageError> {
        let result_pointer = arch::raw_syscall(SyscallNum::IpcReadMessage as usize, message_handle.pack(), dst.as_mut_ptr() as usize, dst.len());
        unsafe { *Box::from_raw(result_pointer as *mut Result<usize, IpcMessageError>) }
    }

    pub fn ipc_receive_message(connection_handle: IpcConnectionHandle) -> FutureHandle {
        let raw = arch::raw_syscall(SyscallNum::IpcReceiveMessage as usize, connection_handle.pack(), 0, 0);
        FutureHandle::unpack(raw)
    }

    pub fn ipc_accept_message(binding_handle: IpcBindingHandle) -> FutureHandle {
        let raw = arch::raw_syscall(SyscallNum::IpcAcceptMessage as usize, binding_handle.pack(), 0, 0);
        FutureHandle::unpack(raw)
    }

    pub fn ipc_dispose_message(message_handle: IpcMessageHandle) -> Result<(), IpcMessageError> {
        let result_pointer = arch::raw_syscall(SyscallNum::IpcDisposeMessage as usize, message_handle.pack(), 0, 0);
        unsafe { *Box::from_raw(result_pointer as *mut Result<(), IpcMessageError>) }
    }
}

