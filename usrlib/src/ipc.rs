use crate::syscall::Syscall;
use core::fmt::{Display, Formatter};
use system::ipc::{IpcConnectionHandle, IpcReceiveError, IpcSendError, MESSAGE_PAYLOAD_BYTES};

pub fn ipc_send_value(connection: IpcConnectionHandle, value: usize) -> Result<(), IpcSendError> {
    let mut data = [0u8; MESSAGE_PAYLOAD_BYTES];
    data[..core::mem::size_of::<usize>()].copy_from_slice(&value.to_ne_bytes());
    Syscall::ipc_send(connection, &data)
}

pub enum ReceiveValueError {
    Receive(IpcReceiveError),
}

impl Display for ReceiveValueError {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        match self {
            ReceiveValueError::Receive(e) => write!(f, "Receive failed: {}", e),
        }
    }
}

impl From<IpcReceiveError> for ReceiveValueError {
    fn from(e: IpcReceiveError) -> Self {
        Self::Receive(e)
    }
}

pub fn ipc_receive_value(connection: IpcConnectionHandle) -> Result<usize, ReceiveValueError> {
    let fh = Syscall::ipc_receive_message(connection);
    let envelope = Syscall::wait_for_message(fh)?;
    let mut bytes = [0u8; core::mem::size_of::<usize>()];
    bytes.copy_from_slice(&envelope.data()[..core::mem::size_of::<usize>()]);
    Ok(usize::from_ne_bytes(bytes))
}
