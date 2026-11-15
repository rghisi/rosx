use crate::wire::{
    self, MAC_BYTES, MAX_ETHERNET_BYTES, MIN_ETHERNET_BYTES, NetError, Reply, Request, Stats,
};
use alloc::format;
use alloc::vec::Vec;
use usrlib::error::DataError;
use usrlib::handle::{UserConnectionHandle, UserDataHandle};
use usrlib::syscall::Syscall;

const CONNECT_ATTEMPTS: usize = 50;
const CONNECT_RETRY_DELAY_MS: u64 = 100;
const DATA_READ_CHUNK_BYTES: usize = 4096;

#[derive(Debug)]
pub enum ClientError {
    Net(NetError),
    Transport,
    NoConnection,
    Data(DataError),
    Protocol,
}

fn data_token(handle: &UserDataHandle) -> u32 {
    u32::from_str_radix(&format!("{}", handle), 16).expect("canonical data token")
}

fn drain_data_object(token: u32) -> Result<Vec<u8>, ClientError> {
    let mut payload = Vec::new();
    loop {
        let mut chunk = [0u8; DATA_READ_CHUNK_BYTES];
        let copied = Syscall::ipc_data_read(token as usize, &mut chunk).map_err(ClientError::Data)?;
        if copied == 0 {
            break;
        }
        payload.extend_from_slice(&chunk[..copied]);
    }
    if let Some(mut object) = UserDataHandle::from_token(token as usize) {
        let _ = object.release();
    }
    Ok(payload)
}

fn transact_frame(connection: UserConnectionHandle, request: Request) -> Result<[u8; wire::FRAME_BYTES], ClientError> {
    let frame = request.encode().map_err(ClientError::Net)?;
    Syscall::ipc_send(connection, &frame).map_err(|_| ClientError::Transport)?;
    let future = Syscall::ipc_receive_message(connection).ok_or(ClientError::Transport)?;
    let received = Syscall::wait_for_message(future).map_err(|_| ClientError::Transport)?;
    if received.connection().is_none() {
        return Err(ClientError::NoConnection);
    }
    Ok(*received.data())
}

pub struct Ne2k {
    connection: UserConnectionHandle,
}

impl Ne2k {
    pub fn connect() -> Result<Ne2k, ClientError> {
        for _ in 0..CONNECT_ATTEMPTS {
            if let Ok(connection) = Syscall::ipc_connect(wire::SERVICE_NAME) {
                return Ok(Ne2k { connection });
            }
            Syscall::sleep(CONNECT_RETRY_DELAY_MS);
        }
        Err(ClientError::NoConnection)
    }

    pub fn mac(&self) -> Result<[u8; MAC_BYTES], ClientError> {
        let frame = transact_frame(self.connection, Request::Mac)?;
        match Reply::decode(&frame).map_err(ClientError::Net)? {
            Reply::Mac(address) => Ok(address),
            Reply::Err { error } => Err(ClientError::Net(error)),
            _ => Err(ClientError::Protocol),
        }
    }

    pub fn send(&self, frame: &[u8]) -> Result<(), ClientError> {
        if frame.len() > MAX_ETHERNET_BYTES {
            return Err(ClientError::Net(NetError::Oversize));
        }
        if frame.len() < MIN_ETHERNET_BYTES {
            return Err(ClientError::Net(NetError::Undersize));
        }
        let mut object =
            Syscall::ipc_data_write(self.connection, frame).map_err(ClientError::Data)?;
        let reply = transact_frame(
            self.connection,
            Request::Send {
                token: data_token(&object),
                length: frame.len() as u16,
            },
        );
        let _ = object.release();
        match Reply::decode(&reply?).map_err(ClientError::Net)? {
            Reply::Ack => Ok(()),
            Reply::Err { error } => Err(ClientError::Net(error)),
            _ => Err(ClientError::Protocol),
        }
    }

    pub fn poll(&self) -> Result<Option<Vec<u8>>, ClientError> {
        let frame = transact_frame(self.connection, Request::Poll)?;
        match Reply::decode(&frame).map_err(ClientError::Net)? {
            Reply::Data { token, length } => {
                let payload = drain_data_object(token)?;
                if payload.len() != length as usize {
                    return Err(ClientError::Protocol);
                }
                Ok(Some(payload))
            }
            Reply::NoFrame => Ok(None),
            Reply::Err { error } => Err(ClientError::Net(error)),
            _ => Err(ClientError::Protocol),
        }
    }

    pub fn stats(&self) -> Result<Stats, ClientError> {
        let frame = transact_frame(self.connection, Request::Stats)?;
        match Reply::decode(&frame).map_err(ClientError::Net)? {
            Reply::Stats(stats) => Ok(stats),
            Reply::Err { error } => Err(ClientError::Net(error)),
            _ => Err(ClientError::Protocol),
        }
    }

    pub fn disconnect(self) {
        Syscall::ipc_disconnect(self.connection);
    }
}
