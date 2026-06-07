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
