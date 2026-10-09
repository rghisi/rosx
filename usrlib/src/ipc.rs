use crate::syscall::Syscall;
use core::fmt::{Display, Formatter};
use system::future::FutureResult;
use system::ipc::{
    IpcConnectionHandle, IpcMessageError, IpcMessageHandle, IpcReceiveError, IpcSendError,
};

pub struct OutgoingMessage {
    message: IpcMessageHandle,
    sent: bool,
}

impl OutgoingMessage {
    pub fn create(connection: IpcConnectionHandle, size: usize) -> Result<Self, IpcSendError> {
        Ok(Self {
            message: Syscall::ipc_create_message(connection, size)?,
            sent: false,
        })
    }

    pub fn write(&mut self, bytes: &[u8]) -> Result<(), IpcMessageError> {
        Syscall::ipc_write_message(self.message, bytes)
    }

    pub fn send(mut self) -> Result<(), IpcSendError> {
        let result = Syscall::ipc_send_message(self.message);
        if result.is_ok() {
            self.sent = true;
        }
        result
    }
}

impl Drop for OutgoingMessage {
    fn drop(&mut self) {
        if !self.sent {
            let _ = Syscall::ipc_dispose_message(self.message);
        }
    }
}

pub struct IncomingMessage {
    message: IpcMessageHandle,
}

impl IncomingMessage {
    pub fn new(message_handle: IpcMessageHandle) -> Self {
        Self { message: message_handle }
    }

    pub fn read(&mut self, dst: &mut [u8]) -> Result<usize, IpcMessageError> {
        Syscall::ipc_read_message(self.message, dst)
    }
}

impl Drop for IncomingMessage {
    fn drop(&mut self) {
        let _ = Syscall::ipc_dispose_message(self.message);
    }
}

pub fn ipc_send_value(connection: IpcConnectionHandle, value: usize) -> Result<(), IpcSendError> {
    let mut message = OutgoingMessage::create(connection, core::mem::size_of::<usize>())?;
    message.write(&value.to_ne_bytes())?;
    message.send()
}

pub enum ReceiveValueError {
    Receive(IpcReceiveError),
    Buffer(IpcMessageError),
    ShortRead,
}

impl Display for ReceiveValueError {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        match self {
            ReceiveValueError::Receive(e) => write!(f, "Receive failed: {}", e),
            ReceiveValueError::Buffer(e) => write!(f, "Buffer failed: {}", e),
            ReceiveValueError::ShortRead => write!(f, "Message shorter than expected"),
        }
    }
}

impl From<IpcReceiveError> for ReceiveValueError {
    fn from(e: IpcReceiveError) -> Self {
        Self::Receive(e)
    }
}

impl From<IpcMessageError> for ReceiveValueError {
    fn from(e: IpcMessageError) -> Self {
        Self::Buffer(e)
    }
}

pub fn ipc_receive_value(connection: IpcConnectionHandle) -> Result<usize, ReceiveValueError> {
    let fh = Syscall::ipc_receive_message(connection);
    match Syscall::wait_future(fh) {
        FutureResult::IpcMessage(Ok(msg)) => {
            let mut message = IncomingMessage::new(msg.message_handle);
            let mut bytes = [0u8; core::mem::size_of::<usize>()];
            let copied = message.read(&mut bytes)?;
            if copied != bytes.len() {
                return Err(ReceiveValueError::ShortRead);
            }
            Ok(usize::from_ne_bytes(bytes))
        }
        FutureResult::IpcMessage(Err(e)) => Err(ReceiveValueError::Receive(e)),
        FutureResult::Void => Err(ReceiveValueError::Receive(
            IpcReceiveError::NoMessagesAvailable,
        )),
    }
}
