use core::fmt::{Display, Formatter};
use collections::generational_arena::Handle;

pub type IpcConnectionHandle = Handle;
pub type IpcBindingHandle = Handle;
pub type IpcMessageHandle = Handle;

pub const MAX_MESSAGE_SIZE: usize = 64;

#[derive(Debug)]
pub enum IpcConnectionError {
    ServerNotFound,
    ConnectionCannotBeEstablished,
}

#[derive(Debug)]
pub enum IpcBindingError {
    AlreadyBound,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IpcSendError {
    ConnectionNotFound,
    ConnectionCongested,
    InvalidBuffer(IpcMessageError),
}

impl Display for IpcSendError {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        match self {
            IpcSendError::ConnectionNotFound =>  write!(f, "Connection not found"),
            IpcSendError::ConnectionCongested => write!(f, "Connection congested"),
            IpcSendError::InvalidBuffer(e) => write!(f, "Invalid buffer: {}", e),
        }
    }
}

impl From<IpcMessageError> for IpcSendError {
    fn from(e: IpcMessageError) -> Self {
        Self::InvalidBuffer(e)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IpcMessageError {
    BufferNotFound,
    Sealed,
    Unsealed,
    PoolExhausted,
    MessageTooLarge,
}

impl Display for IpcMessageError {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        match self {
            IpcMessageError::BufferNotFound => write!(f, "Buffer not found"),
            IpcMessageError::Sealed => write!(f, "Buffer is sealed"),
            IpcMessageError::Unsealed => write!(f, "Buffer is not sealed"),
            IpcMessageError::PoolExhausted => write!(f, "Buffer pool exhausted"),
            IpcMessageError::MessageTooLarge => write!(f, "Message too large"),
        }
    }
}

#[derive(Debug, Clone)]
pub enum IpcReceiveError {
    ConnectionNotFound,
    NoMessagesAvailable,
    MailboxNotAvailable,
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

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct IpcMessage {
    pub message_handle: IpcMessageHandle,
    pub connection_handle: IpcConnectionHandle,
}

use alloc::boxed::Box;
use core::any::Any;
use crate::future::{Future, FutureResult};

pub struct IpcMessageFuture {
    message: Option<IpcMessage>,
    error: Option<IpcReceiveError>,
}

impl IpcMessageFuture {
    pub fn new() -> Self {
        Self { message: None, error: None }
    }

    pub fn with_message(message: IpcMessage) -> Self {
        Self { message: Some(message), error: None }
    }

    pub fn with_error(error: IpcReceiveError) -> Self {
        Self { message: None, error: Some(error) }
    }

    pub fn complete(&mut self, message: IpcMessage) {
        self.message = Some(message);
    }

    pub fn result(&self) -> Result<IpcMessage, IpcReceiveError> {
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

    fn into_result(self: Box<Self>) -> FutureResult {
        FutureResult::IpcMessage(self.result())
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
