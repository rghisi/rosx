use core::fmt::{Display, Formatter};
use collections::generational_arena::Handle;

pub type IpcConnectionHandle = Handle;

#[derive(Debug)]
pub enum IpcConnectionError {
    ServerNotFound,
    ConnectionCannotBeEstablished,
}

pub enum IpcBindingError {
    AlreadyBound,
}

#[derive(Debug)]
pub enum IpcSendError {
    ConnectionNotFound,
    ConnectionCongested
}

impl Display for IpcSendError {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        match self {
            IpcSendError::ConnectionNotFound =>  write!(f, "Connection not found"),
            IpcSendError::ConnectionCongested => write!(f, "Connection congested"),
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
    pub data: usize,
    pub connection_handle: IpcConnectionHandle,
}

use alloc::boxed::Box;
use core::any::Any;
use crate::future::Future;

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
