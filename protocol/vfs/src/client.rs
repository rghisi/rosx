use crate::wire::{self, Reply, Request, VfsError};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::cmp::min;
use usrlib::error::DataError;
use usrlib::handle::{UserConnectionHandle, UserDataHandle};
use usrlib::syscall::Syscall;

pub use crate::wire::{MODE_CREATE, MODE_WRITE};

const CONNECT_ATTEMPTS: usize = 50;
const CONNECT_RETRY_DELAY_MS: u64 = 100;
const DATA_READ_CHUNK_BYTES: usize = 4096;

#[derive(Debug)]
pub enum ClientError {
    Vfs(VfsError),
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
            return Ok(payload);
        }
        payload.extend_from_slice(&chunk[..copied]);
    }
}

fn transact(
    connection: UserConnectionHandle,
    request: Request<'_>,
) -> Result<Reply, ClientError> {
    let frame = request.encode().map_err(ClientError::Vfs)?;
    Syscall::ipc_send(connection, &frame).map_err(|_| ClientError::Transport)?;
    let future = Syscall::ipc_receive_message(connection).ok_or(ClientError::Transport)?;
    let received = Syscall::wait_for_message(future).map_err(|_| ClientError::Transport)?;
    if received.connection().is_none() {
        return Err(ClientError::NoConnection);
    }
    Reply::decode(received.data()).map_err(|_| ClientError::Protocol)
}

pub struct Vfs {
    connection: UserConnectionHandle,
}

impl Vfs {
    pub fn connect() -> Result<Vfs, ClientError> {
        for _ in 0..CONNECT_ATTEMPTS {
            if let Ok(connection) = Syscall::ipc_connect(wire::SERVICE_NAME) {
                return Ok(Vfs { connection });
            }
            Syscall::sleep(CONNECT_RETRY_DELAY_MS);
        }
        Err(ClientError::NoConnection)
    }

    pub fn open(&self, name: &str, mode: u8) -> Result<VfsFile, ClientError> {
        let request = wire::Request::Open { mode, name };
        match transact(self.connection, request)? {
            Reply::Opened { handle } => Ok(VfsFile {
                connection: self.connection,
                handle,
                offset: 0,
            }),
            Reply::Err(error) => Err(ClientError::Vfs(error)),
            _ => Err(ClientError::Protocol),
        }
    }

    pub fn delete(&self, name: &str) -> Result<(), ClientError> {
        let request = wire::Request::Delete { name };
        match transact(self.connection, request)? {
            Reply::Ack => Ok(()),
            Reply::Err(error) => Err(ClientError::Vfs(error)),
            _ => Err(ClientError::Protocol),
        }
    }

    pub fn list(&self) -> Result<Vec<String>, ClientError> {
        let request = wire::Request::List;
        match transact(self.connection, request)? {
            Reply::Eof => Ok(Vec::new()),
            Reply::Listed { token, total } => {
                let payload = drain_data_object(token)?;
                if payload.len() as u32 != total {
                    return Err(ClientError::Protocol);
                }
                let text = core::str::from_utf8(&payload).map_err(|_| ClientError::Protocol)?;
                Ok(text.split('\n').map(String::from).collect())
            }
            Reply::Err(error) => Err(ClientError::Vfs(error)),
            _ => Err(ClientError::Protocol),
        }
    }

    pub fn disconnect(self) {
        Syscall::ipc_disconnect(self.connection);
    }
}

pub struct VfsFile {
    connection: UserConnectionHandle,
    handle: u32,
    offset: u64,
}

impl VfsFile {
    pub fn read(&mut self, buf: &mut [u8]) -> Result<usize, ClientError> {
        let mut total = 0usize;
        while total < buf.len() {
            let requested = min(buf.len() - total, wire::RPC_MAX_DATA_BYTES) as u32;
            let request = wire::Request::Read {
                handle: self.handle,
                offset: self.offset,
                count: requested,
            };
            match transact(self.connection, request)? {
                Reply::Eof => break,
                Reply::ReadData { token, count } => {
                    let chunk = drain_data_object(token)?;
                    if chunk.len() as u32 != count {
                        return Err(ClientError::Protocol);
                    }
                    buf[total..total + chunk.len()].copy_from_slice(&chunk);
                    total += chunk.len();
                    self.offset += chunk.len() as u64;
                }
                Reply::Err(error) => return Err(ClientError::Vfs(error)),
                _ => return Err(ClientError::Protocol),
            }
        }
        Ok(total)
    }

    pub fn write(&mut self, data: &[u8]) -> Result<(), ClientError> {
        let mut written = 0usize;
        loop {
            let end = min(written + wire::RPC_MAX_DATA_BYTES, data.len());
            let chunk = &data[written..end];
            let mut object = Syscall::ipc_data_write(self.connection, chunk).map_err(ClientError::Data)?;
            let request = wire::Request::Write {
                handle: self.handle,
                offset: self.offset,
                token: data_token(&object),
            };
            let reply = transact(self.connection, request);
            let _ = object.release();
            match reply? {
                Reply::Written { bytes } => {
                    if bytes as usize != chunk.len() {
                        return Err(ClientError::Protocol);
                    }
                    self.offset += bytes as u64;
                    written = end;
                }
                Reply::Err(error) => return Err(ClientError::Vfs(error)),
                _ => return Err(ClientError::Protocol),
            }
            if written >= data.len() {
                return Ok(());
            }
        }
    }

    pub fn seek(&mut self, offset: u64) {
        self.offset = offset;
    }

    pub fn offset(&self) -> u64 {
        self.offset
    }

    pub fn close(self) -> Result<(), ClientError> {
        let request = wire::Request::Close { handle: self.handle };
        match transact(self.connection, request)? {
            Reply::Ack => Ok(()),
            Reply::Err(error) => Err(ClientError::Vfs(error)),
            _ => Err(ClientError::Protocol),
        }
    }
}
