use crate::error::DataError;
use crate::error::IpcReceiveError;
use crate::error::IpcSendError;
use crate::handle::{UserConnectionHandle, UserDataHandle};
use crate::syscall::Syscall;
use alloc::vec::Vec;
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

const DATA_TOKEN_BYTES: usize = core::mem::size_of::<u32>();
const RECEIVE_CHUNK_BYTES: usize = 4096;

pub(crate) fn encode_data_token(data: &mut [u8; MESSAGE_PAYLOAD_BYTES], token: u32) {
    data[..DATA_TOKEN_BYTES].copy_from_slice(&token.to_ne_bytes());
}

pub(crate) fn decode_data_token(data: &[u8; MESSAGE_PAYLOAD_BYTES]) -> u32 {
    let mut bytes = [0u8; DATA_TOKEN_BYTES];
    bytes.copy_from_slice(&data[..DATA_TOKEN_BYTES]);
    u32::from_ne_bytes(bytes)
}

pub enum SendDataError {
    Data(DataError),
    Send(IpcSendError),
}

impl Display for SendDataError {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        match self {
            SendDataError::Data(e) => write!(f, "Data write failed: {}", e),
            SendDataError::Send(e) => write!(f, "Data signal send failed: {}", e),
        }
    }
}

impl From<DataError> for SendDataError {
    fn from(e: DataError) -> Self {
        Self::Data(e)
    }
}

impl From<IpcSendError> for SendDataError {
    fn from(e: IpcSendError) -> Self {
        Self::Send(e)
    }
}

pub fn send_data(
    connection: UserConnectionHandle,
    payload: &[u8],
) -> Result<UserDataHandle, SendDataError> {
    let handle = Syscall::ipc_data_write(connection, payload)?;
    let mut data = [0u8; MESSAGE_PAYLOAD_BYTES];
    encode_data_token(&mut data, handle.packed());
    Syscall::ipc_send(connection, &data).map_err(SendDataError::Send)?;
    Ok(handle)
}

pub fn receive_data(message: &ReceivedMessage) -> Result<Vec<u8>, DataError> {
    let token = decode_data_token(message.data());
    let mut payload = Vec::new();
    loop {
        let mut chunk = [0u8; RECEIVE_CHUNK_BYTES];
        let copied = Syscall::ipc_data_read(token as usize, &mut chunk)?;
        if copied == 0 {
            return Ok(payload);
        }
        payload.extend_from_slice(&chunk[..copied]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_token_codec_roundtrips_at_payload_offset_zero() {
        let mut data = [0u8; MESSAGE_PAYLOAD_BYTES];
        encode_data_token(&mut data, 0x00AB_CDEF);
        assert_eq!(decode_data_token(&data), 0x00AB_CDEF);
    }

    #[test]
    fn data_token_occupies_only_the_first_four_payload_bytes() {
        let mut data = [0u8; MESSAGE_PAYLOAD_BYTES];
        encode_data_token(&mut data, 0x00AB_CDEF);
        assert_eq!(&data[..DATA_TOKEN_BYTES], &0x00AB_CDEFu32.to_ne_bytes());
        assert!(data[DATA_TOKEN_BYTES..].iter().all(|byte| *byte == 0));
    }

    #[test]
    fn zero_token_encodes_and_decodes_to_zero() {
        let mut data = [0u8; MESSAGE_PAYLOAD_BYTES];
        encode_data_token(&mut data, 0);
        assert_eq!(decode_data_token(&data), 0);
    }
}
