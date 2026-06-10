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

#[derive(Debug)]
pub enum IpcReceiveError {
    ConnectionNotFound,
    NoMessagesAvailable
}

impl Display for IpcReceiveError {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        match self {
            IpcReceiveError::ConnectionNotFound => write!(f, "Connection not found"),
            IpcReceiveError::NoMessagesAvailable => write!(f, "No messages available"),
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct IpcMessage {
    pub data: usize,
    pub connection_handle: IpcConnectionHandle,
}

use core::any::Any;
use crate::future::{Future, FutureHandle};

#[derive(Debug, PartialEq)]
pub enum ReceiveOutcome {
    Ready(IpcMessage),
    Pending(Handle),
}

pub struct IpcMessageFuture {
    message: Option<IpcMessage>,
    handle: Option<FutureHandle>,
}

impl IpcMessageFuture {
    pub fn new() -> Self {
        Self { message: None, handle: None }
    }

    pub fn with_message(message: IpcMessage) -> Self {
        Self { message: Some(message), handle: None }
    }

    pub fn with_handle(handle: FutureHandle) -> Self {
        Self { message: None, handle: Some(handle) }
    }

    pub fn complete(&mut self, message: IpcMessage) {
        self.message = Some(message);
    }

    pub fn get_message(&self) -> Option<IpcMessage> {
        self.message
    }

    pub fn get_handle(&self) -> Option<FutureHandle> {
        self.handle
    }
}

impl Future for IpcMessageFuture {
    fn is_completed(&self) -> bool {
        self.message.is_some()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}
