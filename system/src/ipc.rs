use core::fmt::{Display, Formatter};
use collections::generational_arena::Handle;
use crate::error::{ErrorCode, ErrorCodeOf};

pub type IpcConnectionHandle = Handle;
pub type IpcBindingHandle = Handle;

pub const MESSAGE_BYTES: usize = 64;
pub const MESSAGE_PAYLOAD_BYTES: usize = 60;

#[repr(C)]
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct Message {
    conn: u32,
    data: [u8; MESSAGE_PAYLOAD_BYTES],
}

impl Message {
    pub const EMPTY: Self = Self { conn: 0, data: [0; MESSAGE_PAYLOAD_BYTES] };

    pub fn new(connection: IpcConnectionHandle, data: &[u8; MESSAGE_PAYLOAD_BYTES]) -> Self {
        Self { conn: connection.pack() as u32, data: *data }
    }

    pub fn conn(&self) -> IpcConnectionHandle {
        Handle::unpack(self.conn as usize)
    }

    pub fn data(&self) -> &[u8; MESSAGE_PAYLOAD_BYTES] {
        &self.data
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum IpcConnectionError {
    ServerNotFound,
    ConnectionCannotBeEstablished,
}

impl IpcConnectionError {
    pub fn to_reg(self) -> usize {
        self.to_error_code().to_reg()
    }

    pub fn from_reg(raw: usize) -> Self {
        match ErrorCode::from_code(raw) {
            ErrorCode::IpcConnectionCannotBeEstablished => Self::ConnectionCannotBeEstablished,
            _ => Self::ServerNotFound,
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum IpcBindingError {
    AlreadyBound,
}

impl IpcBindingError {
    pub fn to_reg(self) -> usize {
        self.to_error_code().to_reg()
    }

    pub fn from_reg(raw: usize) -> Self {
        match ErrorCode::from_code(raw) {
            _ => IpcBindingError::AlreadyBound,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IpcSendError {
    ConnectionNotFound,
    ConnectionCongested,
}

impl IpcSendError {
    pub fn to_reg(self) -> usize {
        self.to_error_code().to_reg()
    }

    pub fn from_reg(raw: usize) -> Self {
        match ErrorCode::from_code(raw) {
            ErrorCode::IpcSendConnectionCongested => IpcSendError::ConnectionCongested,
            _ => IpcSendError::ConnectionNotFound,
        }
    }
}

impl Display for IpcSendError {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        match self {
            IpcSendError::ConnectionNotFound =>  write!(f, "Connection not found"),
            IpcSendError::ConnectionCongested => write!(f, "Connection congested"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IpcReceiveError {
    ConnectionNotFound,
    NoMessagesAvailable,
    MailboxNotAvailable,
}

impl IpcReceiveError {
    pub fn to_reg(self) -> usize {
        self.to_error_code().to_reg()
    }

    pub fn from_reg(raw: usize) -> Self {
        match ErrorCode::from_code(raw) {
            ErrorCode::IpcReceiveNoMessagesAvailable => IpcReceiveError::NoMessagesAvailable,
            ErrorCode::IpcMailboxNotAvailable => IpcReceiveError::MailboxNotAvailable,
            _ => IpcReceiveError::ConnectionNotFound,
        }
    }
}

impl Display for IpcReceiveError {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        match self {
            IpcReceiveError::ConnectionNotFound => write!(f, "Connection not found"),
            IpcReceiveError::NoMessagesAvailable => write!(f, "No messages available"),
            IpcReceiveError::MailboxNotAvailable => write!(f, "Mailbox not available"),
        }
    }
}

use alloc::boxed::Box;
use core::any::Any;
use crate::future::Future;

pub struct IpcMessageFuture {
    message: Option<Message>,
    error: Option<IpcReceiveError>,
}

impl IpcMessageFuture {
    pub fn new() -> Self {
        Self { message: None, error: None }
    }

    pub fn with_message(message: Message) -> Self {
        Self { message: Some(message), error: None }
    }

    pub fn with_error(error: IpcReceiveError) -> Self {
        Self { message: None, error: Some(error) }
    }

    pub fn complete(&mut self, message: Message) {
        self.message = Some(message);
    }

    pub fn result(&self) -> Result<Message, IpcReceiveError> {
        if let Some(err) = self.error.clone() {
            return Err(err);
        }
        self.message.ok_or(IpcReceiveError::NoMessagesAvailable)
    }
}

impl Future for IpcMessageFuture {
    fn is_completed(&self) -> bool {
        self.message.is_some() || self.error.is_some()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn into_any(self: Box<Self>) -> Box<dyn Any + Send + Sync> {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use collections::generational_arena::ERROR_BIT;

    #[test]
    fn message_is_64_bytes_align_4() {
        assert_eq!(core::mem::size_of::<Message>(), 64);
        assert_eq!(core::mem::align_of::<Message>(), 4);
    }

    #[test]
    fn message_conn_field_at_offset_zero() {
        assert_eq!(core::mem::offset_of!(Message, conn), 0);
        assert_eq!(core::mem::offset_of!(Message, data), 4);
    }

    #[test]
    fn message_new_and_accessors_roundtrip() {
        let extreme = Handle::new(0xFFFF, 0x7FFF);
        let pattern: [u8; MESSAGE_PAYLOAD_BYTES] = core::array::from_fn(|i| i as u8);
        let message = Message::new(extreme, &pattern);
        assert_eq!(message.conn(), extreme);
        assert_eq!(*message.data(), pattern);
    }

    #[test]
    fn packed_handle_space_is_disjoint_from_error_space() {
        assert_eq!(Handle::new(0xFFFF, 0x7FFF).pack(), 0x7FFF_FFFF);
        assert!(Handle::is_handle(0x7FFF_FFFF));
        assert!(!Handle::is_handle(ERROR_BIT));
        for error in [IpcConnectionError::ServerNotFound, IpcConnectionError::ConnectionCannotBeEstablished] {
            let raw = error.to_reg();
            assert_ne!(raw & ERROR_BIT, 0);
            assert!(!Handle::is_handle(raw));
            assert_eq!(IpcConnectionError::from_reg(raw), error);
        }
        for error in [IpcBindingError::AlreadyBound] {
            let raw = error.to_reg();
            assert_ne!(raw & ERROR_BIT, 0);
            assert!(!Handle::is_handle(raw));
            assert_eq!(IpcBindingError::from_reg(raw), error);
        }
        for error in [IpcSendError::ConnectionNotFound, IpcSendError::ConnectionCongested] {
            let raw = error.clone().to_reg();
            assert_ne!(raw & ERROR_BIT, 0);
            assert!(!Handle::is_handle(raw));
            assert_eq!(IpcSendError::from_reg(raw), error);
        }
        for error in [
            IpcReceiveError::ConnectionNotFound,
            IpcReceiveError::NoMessagesAvailable,
            IpcReceiveError::MailboxNotAvailable,
        ] {
            let raw = error.to_reg();
            assert_ne!(raw & ERROR_BIT, 0);
            assert!(!Handle::is_handle(raw));
            assert_eq!(IpcReceiveError::from_reg(raw), error);
        }
    }
}
