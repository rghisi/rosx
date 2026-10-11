use crate::error::IpcReceiveError;
use crate::error::IpcSendError;
use crate::handle::UserConnectionHandle;
use crate::syscall::Syscall;
use core::fmt::{Display, Formatter};
use system::ipc::{MESSAGE_PAYLOAD_BYTES, Message};

pub struct ReceivedMessage {
    message: Message,
}

impl ReceivedMessage {
    pub(crate) fn new(message: Message) -> Self {
        Self { message }
    }

    pub fn connection(&self) -> Option<UserConnectionHandle> {
        UserConnectionHandle::from_token(self.message.conn() as usize)
    }

    pub fn data(&self) -> &[u8; MESSAGE_PAYLOAD_BYTES] {
        self.message.data()
    }
}

pub fn ipc_send_value(connection: UserConnectionHandle, value: usize) -> Result<(), IpcSendError> {
    let mut data = [0u8; MESSAGE_PAYLOAD_BYTES];
    data[..core::mem::size_of::<usize>()].copy_from_slice(&value.to_ne_bytes());
    Syscall::ipc_send(connection, &data)
}

pub enum ReceiveValueError {
    Receive(IpcReceiveError),
    NoHandle,
}

impl Display for ReceiveValueError {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        match self {
            ReceiveValueError::Receive(e) => write!(f, "Receive failed: {}", e),
            ReceiveValueError::NoHandle => write!(f, "No handle returned"),
        }
    }
}

impl From<IpcReceiveError> for ReceiveValueError {
    fn from(e: IpcReceiveError) -> Self {
        Self::Receive(e)
    }
}

pub fn ipc_receive_value(connection: UserConnectionHandle) -> Result<usize, ReceiveValueError> {
    let Some(fh) = Syscall::ipc_receive_message(connection) else {
        return Err(ReceiveValueError::NoHandle);
    };
    let envelope = Syscall::wait_for_message(fh)?;
    let mut bytes = [0u8; core::mem::size_of::<usize>()];
    bytes.copy_from_slice(&envelope.data()[..core::mem::size_of::<usize>()]);
    Ok(usize::from_ne_bytes(bytes))
}
