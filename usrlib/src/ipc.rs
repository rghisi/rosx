use crate::syscall::Syscall;
use core::fmt::{Display, Formatter};
use system::future::FutureResult;
use system::ipc::{
    IpcBufferError, IpcBufferHandle, IpcConnectionHandle, IpcReceiveError, IpcSendError,
};

pub struct WritableMailbox {
    buffer: IpcBufferHandle,
    sent: bool,
}

impl WritableMailbox {
    pub fn alloc() -> Result<Self, IpcBufferError> {
        Ok(Self {
            buffer: Syscall::ipc_alloc_buffer()?,
            sent: false,
        })
    }

    pub fn write(&mut self, bytes: &[u8]) -> Result<(), IpcBufferError> {
        Syscall::ipc_write_buffer(self.buffer, bytes)
    }

    pub fn send(mut self, connection: IpcConnectionHandle) -> Result<(), IpcSendError> {
        let result = Syscall::ipc_send(connection, self.buffer);
        if result.is_ok() {
            self.sent = true;
        }
        result
    }

    pub fn send_to_client(mut self, connection: IpcConnectionHandle) -> Result<(), IpcSendError> {
        let result = Syscall::ipc_send_to_client(connection, self.buffer);
        if result.is_ok() {
            self.sent = true;
        }
        result
    }
}

impl Drop for WritableMailbox {
    fn drop(&mut self) {
        if !self.sent {
            let _ = Syscall::ipc_dispose_buffer(self.buffer);
        }
    }
}

pub struct ReadableMailbox {
    buffer: IpcBufferHandle,
}

impl ReadableMailbox {
    pub fn new(buffer: IpcBufferHandle) -> Self {
        Self { buffer }
    }

    pub fn read(&mut self, dst: &mut [u8]) -> Result<usize, IpcBufferError> {
        Syscall::ipc_read_buffer(self.buffer, dst)
    }
}

impl Drop for ReadableMailbox {
    fn drop(&mut self) {
        let _ = Syscall::ipc_dispose_buffer(self.buffer);
    }
}

pub fn ipc_send_value(connection: IpcConnectionHandle, value: usize) -> Result<(), IpcSendError> {
    let mut mailbox = WritableMailbox::alloc()?;
    mailbox.write(&value.to_ne_bytes())?;
    mailbox.send(connection)
}

pub fn ipc_send_value_to_client(
    connection: IpcConnectionHandle,
    value: usize,
) -> Result<(), IpcSendError> {
    let mut mailbox = WritableMailbox::alloc()?;
    mailbox.write(&value.to_ne_bytes())?;
    mailbox.send_to_client(connection)
}

pub enum ReceiveValueError {
    Receive(IpcReceiveError),
    Buffer(IpcBufferError),
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

impl From<IpcBufferError> for ReceiveValueError {
    fn from(e: IpcBufferError) -> Self {
        Self::Buffer(e)
    }
}

pub fn ipc_receive_value(connection: IpcConnectionHandle) -> Result<usize, ReceiveValueError> {
    let fh = Syscall::ipc_receive(connection);
    match Syscall::wait_future(fh) {
        FutureResult::IpcMessage(Ok(msg)) => {
            let mut mailbox = ReadableMailbox::new(msg.buffer_handle);
            let mut bytes = [0u8; core::mem::size_of::<usize>()];
            let copied = mailbox.read(&mut bytes)?;
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
