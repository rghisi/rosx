use crate::arch;
use crate::error::{
    DataError, IpcBindingError, IpcConnectionError, IpcReceiveError, IpcSendError, PortError,
    WaitsetError,
};
use crate::handle::{UserBindingHandle, UserConnectionHandle, UserFutureHandle, UserDataHandle};
use crate::ipc::ReceivedMessage;
use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;
use system::error::{ErrorCode, is_token};
use system::ipc::{MESSAGE_PAYLOAD_BYTES, Message};
use system::syscall_numbers::SyscallNum;
use system::waitset::WaitEvent;

pub struct Syscall {}

impl Syscall {
    pub fn exec(entrypoint: usize) -> Option<UserFutureHandle> {
        let raw = arch::raw_syscall(SyscallNum::Exec as usize, entrypoint as usize, 0, 0);
        UserFutureHandle::from_token(raw)
    }

    pub fn load(elf: &[u8]) -> Option<UserFutureHandle> {
        let elf_ptr = Box::into_raw(Box::new(elf)) as usize;
        let raw = arch::raw_syscall(SyscallNum::LoadElf as usize, elf_ptr as usize, 0, 0);
        UserFutureHandle::from_token(raw)
    }

    pub fn load_with_args(elf: &[u8], args: &[String]) -> Option<UserFutureHandle> {
        let elf_ptr = Box::into_raw(Box::new(elf)) as usize;
        let blob = encode_arguments(args);
        let (blob_ptr, blob_len) = wire_pair(&blob);
        let raw = arch::raw_syscall(SyscallNum::LoadElf as usize, elf_ptr, blob_ptr, blob_len);
        UserFutureHandle::from_token(raw)
    }

    pub fn task_yield() {
        arch::raw_syscall(SyscallNum::Yield as usize, 0, 0, 0);
    }

    pub fn sleep(ms: u64) {
        arch::raw_syscall(SyscallNum::Sleep as usize, ms as usize, 0, 0);
    }

    pub fn now() -> u64 {
        arch::raw_syscall(SyscallNum::Now as usize, 0, 0, 0) as u64
    }

    pub fn wait_future(handle: UserFutureHandle) {
        arch::raw_syscall(
            SyscallNum::WaitFuture as usize,
            handle.value(),
            0,
            0,
        );
    }

    pub fn wait_for_message(handle: UserFutureHandle) -> Result<ReceivedMessage, IpcReceiveError> {
        let mut envelope = Message::EMPTY;
        let raw = arch::raw_syscall(
            SyscallNum::WaitFuture as usize,
            handle.value(),
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
            handle.value(),
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
        crate::input::read_char()
    }

    pub fn try_read_char() -> Option<char> {
        crate::input::try_read_char()
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
            connection_handle.value(),
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
        let envelope = Message::new(connection.value() as u32, data);
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
            connection_handle.value(),
            0,
            0,
        );
        UserFutureHandle::from_token(raw)
    }

    pub fn ipc_accept_message(binding_handle: UserBindingHandle) -> Option<UserFutureHandle> {
        let raw = arch::raw_syscall(
            SyscallNum::IpcAcceptMessage as usize,
            binding_handle.value(),
            0,
            0,
        );
        UserFutureHandle::from_token(raw)
    }

    pub fn ipc_data_write(
        connection: UserConnectionHandle,
        payload: &[u8],
    ) -> Result<UserDataHandle, DataError> {
        let raw = arch::raw_syscall(
            SyscallNum::IpcDataWrite as usize,
            connection.value(),
            payload.as_ptr() as usize,
            payload.len(),
        );
        match UserDataHandle::from_token(raw) {
            Some(handle) => Ok(handle),
            None => Err(DataError::from(ErrorCode::from_code(raw))),
        }
    }

    pub fn ipc_data_read(token: usize, dst: &mut [u8]) -> Result<usize, DataError> {
        let raw = arch::raw_syscall(
            SyscallNum::IpcDataRead as usize,
            token,
            dst.as_mut_ptr() as usize,
            dst.len(),
        );
        if is_token(raw) {
            Ok(raw)
        } else {
            Err(DataError::from(ErrorCode::from_code(raw)))
        }
    }

    pub fn rom_read(offset: u64, dst: &mut [u8]) -> usize {
        arch::raw_syscall(
            SyscallNum::RomRead as usize,
            offset as usize,
            dst.as_mut_ptr() as usize,
            dst.len(),
        )
    }

    pub fn waitset_create() -> Result<UserFutureHandle, WaitsetError> {
        let raw = arch::raw_syscall(SyscallNum::WaitsetCreate as usize, 0, 0, 0);
        match UserFutureHandle::from_token(raw) {
            Some(handle) => Ok(handle),
            None => Err(WaitsetError::from(ErrorCode::from_code(raw))),
        }
    }

    pub fn waitset_attach(
        set: UserFutureHandle,
        child: UserFutureHandle,
        tag: u32,
    ) -> Result<(), WaitsetError> {
        let raw = arch::raw_syscall(
            SyscallNum::WaitsetAttach as usize,
            set.value(),
            child.value(),
            tag as usize,
        );
        if is_token(raw) {
            Ok(())
        } else {
            Err(WaitsetError::from(ErrorCode::from_code(raw)))
        }
    }

    pub fn waitset_detach(
        set: UserFutureHandle,
        child: UserFutureHandle,
    ) -> Result<(), WaitsetError> {
        let raw = arch::raw_syscall(
            SyscallNum::WaitsetDetach as usize,
            set.value(),
            child.value(),
            0,
        );
        if is_token(raw) {
            Ok(())
        } else {
            Err(WaitsetError::from(ErrorCode::from_code(raw)))
        }
    }

    pub fn waitset_wait(set: UserFutureHandle) -> Result<(UserFutureHandle, u32), WaitsetError> {
        let mut event = WaitEvent { child: 0, tag: 0 };
        let raw = arch::raw_syscall(
            SyscallNum::WaitsetWait as usize,
            set.value(),
            &mut event as *mut WaitEvent as usize,
            0,
        );
        if is_token(raw) {
            match UserFutureHandle::from_token(event.child as usize) {
                Some(child) => Ok((child, event.tag)),
                None => Err(WaitsetError::ChildInvalid),
            }
        } else {
            Err(WaitsetError::from(ErrorCode::from_code(raw)))
        }
    }

    pub fn waitset_drop(set: UserFutureHandle) {
        arch::raw_syscall(SyscallNum::WaitsetDrop as usize, set.value(), 0, 0);
    }

    pub fn port_in(port: u16, width: usize) -> Result<u32, PortError> {
        let mut slot: u32 = 0;
        let raw = arch::raw_syscall(
            SyscallNum::PortIn as usize,
            port as usize,
            width,
            &mut slot as *mut u32 as usize,
        );
        if is_token(raw) {
            Ok(slot)
        } else {
            Err(PortError::from(ErrorCode::from_code(raw)))
        }
    }

    pub fn port_out(port: u16, value: usize, width: usize) -> Result<(), PortError> {
        let raw = arch::raw_syscall(SyscallNum::PortOut as usize, port as usize, value, width);
        if is_token(raw) {
            Ok(())
        } else {
            Err(PortError::from(ErrorCode::from_code(raw)))
        }
    }

    pub fn port_in_words(port: u16, destination: &mut [u16]) -> Result<(), PortError> {
        let raw = arch::raw_syscall(
            SyscallNum::PortInBlock as usize,
            port as usize,
            destination.as_mut_ptr() as usize,
            destination.len(),
        );
        if is_token(raw) {
            Ok(())
        } else {
            Err(PortError::from(ErrorCode::from_code(raw)))
        }
    }

    pub fn port_out_words(port: u16, source: &[u16]) -> Result<(), PortError> {
        let raw = arch::raw_syscall(
            SyscallNum::PortOutBlock as usize,
            port as usize,
            source.as_ptr() as usize,
            source.len(),
        );
        if is_token(raw) {
            Ok(())
        } else {
            Err(PortError::from(ErrorCode::from_code(raw)))
        }
    }
}

fn encode_arguments(args: &[String]) -> Vec<u8> {
    let mut blob = Vec::new();
    for argument in args {
        blob.extend_from_slice(argument.as_bytes());
        blob.push(0);
    }
    blob
}

fn wire_pair(blob: &[u8]) -> (usize, usize) {
    if blob.is_empty() {
        (0, 0)
    } else {
        (blob.as_ptr() as usize, blob.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_arguments_encodes_to_empty_blob_and_zero_zero_wire_pair() {
        let blob = encode_arguments(&[]);
        assert!(blob.is_empty());
        assert_eq!(wire_pair(&blob), (0, 0));
    }

    #[test]
    fn non_empty_blob_pairs_a_live_pointer_with_its_full_length() {
        let blob = encode_arguments(&[String::from("a")]);
        let (blob_ptr, blob_len) = wire_pair(&blob);
        assert_ne!(blob_ptr, 0);
        assert_ne!(blob_len, 0);
        assert_eq!(blob_len, blob.len());
    }

    #[test]
    fn two_arguments_encode_with_one_terminator_each() {
        let args = [String::from("a"), String::from("b")];
        assert_eq!(encode_arguments(&args), *b"a\0b\0");
    }

    #[test]
    fn encoded_size_is_argument_bytes_plus_argument_count() {
        let args = [
            String::from("pi"),
            String::from("alpha"),
            String::from("beta"),
        ];
        let blob = encode_arguments(&args);
        assert_eq!(blob, *b"pi\0alpha\0beta\0");
        let expected: usize = args.iter().map(String::len).sum::<usize>() + args.len();
        assert_eq!(blob.len(), expected);
        assert_eq!(blob.len(), 2 + 5 + 4 + 3);
    }
}
