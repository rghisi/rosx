use crate::arch;
use crate::error::{IpcBindingError, IpcConnectionError, IpcReceiveError, IpcSendError};
use crate::handle::{UserBindingHandle, UserConnectionHandle, UserFutureHandle};
use crate::ipc::ReceivedMessage;
use alloc::boxed::Box;
use core::fmt;
use system::error::{ErrorCode, is_token};
use system::ipc::{MESSAGE_PAYLOAD_BYTES, Message};
use system::syscall_numbers::SyscallNum;

pub struct Syscall {}

impl Syscall {
    pub fn exec(entrypoint: usize) -> Option<UserFutureHandle> {
        let raw = arch::raw_syscall(SyscallNum::Exec as usize, entrypoint as usize, 0, 0);
        UserFutureHandle::from_token(raw)
    }

    pub fn load(elf: &'static [u8]) -> Option<UserFutureHandle> {
        let elf_ptr = Box::into_raw(Box::new(elf)) as usize;
        let raw = arch::raw_syscall(SyscallNum::LoadElf as usize, elf_ptr as usize, 0, 0);
        UserFutureHandle::from_token(raw)
    }

    pub fn task_yield() {
        arch::raw_syscall(SyscallNum::Yield as usize, 0, 0, 0);
    }

    pub fn sleep(ms: u64) {
        arch::raw_syscall(SyscallNum::Sleep as usize, ms as usize, 0, 0);
    }

    pub fn wait_future(handle: UserFutureHandle) {
        arch::raw_syscall(
            SyscallNum::WaitFuture as usize,
            handle.packed() as usize,
            0,
            0,
        );
    }

    pub fn wait_for_message(handle: UserFutureHandle) -> Result<ReceivedMessage, IpcReceiveError> {
        let mut envelope = Message::EMPTY;
        let raw = arch::raw_syscall(
            SyscallNum::WaitFuture as usize,
            handle.packed() as usize,
            &mut envelope as *mut Message as usize,
            0,
        );
        if is_token(raw) {
            Ok(ReceivedMessage::new(envelope))
        } else {
            Err(IpcReceiveError::from(ErrorCode::from_code(raw)))
        }
    }

    pub fn is_future_completed(handle: UserFutureHandle) -> bool {
        let result = arch::raw_syscall(
            SyscallNum::IsFutureCompleted as usize,
            handle.packed() as usize,
            0,
            0,
        );
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

    pub fn ipc_connect(service: &str) -> Result<UserConnectionHandle, IpcConnectionError> {
        let boxed = Box::into_raw(Box::new(service)) as usize;
        let raw = arch::raw_syscall(SyscallNum::IpcConnect as usize, boxed, 0, 0);
        match UserConnectionHandle::from_token(raw) {
            Some(handle) => Ok(handle),
            None => Err(IpcConnectionError::from(ErrorCode::from_code(raw))),
        }
    }

    pub fn ipc_disconnect(connection_handle: UserConnectionHandle) {
        arch::raw_syscall(
            SyscallNum::IpcDisconnect as usize,
            connection_handle.packed() as usize,
            0,
            0,
        );
    }

    pub fn ipc_bind(service: &str) -> Result<UserBindingHandle, IpcBindingError> {
        let boxed = Box::into_raw(Box::new(service)) as usize;
        let raw = arch::raw_syscall(SyscallNum::IpcBind as usize, boxed, 0, 0);
        match UserBindingHandle::from_token(raw) {
            Some(handle) => Ok(handle),
            None => Err(IpcBindingError::from(ErrorCode::from_code(raw))),
        }
    }

    pub fn ipc_send(
        connection: UserConnectionHandle,
        data: &[u8; MESSAGE_PAYLOAD_BYTES],
    ) -> Result<(), IpcSendError> {
        let envelope = Message::new(connection.packed(), data);
        let raw = arch::raw_syscall(
            SyscallNum::IpcSendMessage as usize,
            &envelope as *const Message as usize,
            0,
            0,
        );
        if is_token(raw) {
            Ok(())
        } else {
            Err(IpcSendError::from(ErrorCode::from_code(raw)))
        }
    }

    pub fn ipc_receive_message(
        connection_handle: UserConnectionHandle,
    ) -> Option<UserFutureHandle> {
        let raw = arch::raw_syscall(
            SyscallNum::IpcReceiveMessage as usize,
            connection_handle.packed() as usize,
            0,
            0,
        );
        UserFutureHandle::from_token(raw)
    }

    pub fn ipc_accept_message(binding_handle: UserBindingHandle) -> Option<UserFutureHandle> {
        let raw = arch::raw_syscall(
            SyscallNum::IpcAcceptMessage as usize,
            binding_handle.packed() as usize,
            0,
            0,
        );
        UserFutureHandle::from_token(raw)
    }
}
