use crate::wire::{self, Expect, Reply, VfsError};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use usrlib::error::DataError;
use usrlib::handle::{UserConnectionHandle, UserDataHandle};
use usrlib::syscall::Syscall;

pub const MODE_WRITE: u8 = 1;
pub const MODE_CREATE: u8 = 2;

const CONNECT_ATTEMPTS: usize = 50;
const CONNECT_RETRY_DELAY_MS: u64 = 100;
const DRAIN_CHUNK_BYTES: usize = 4096;

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum ClientError {
    Vfs(VfsError),
    Transport,
    NoConnection,
    Data(DataError),
    Protocol,
}

pub struct Vfs {
    connection: UserConnectionHandle,
}

impl Vfs {
    pub fn connect() -> Result<Vfs, ClientError> {
        let mut attempt = 0;
        loop {
            if let Ok(connection) = Syscall::ipc_connect("VFS") {
                return Ok(Vfs { connection });
            }
            attempt += 1;
            if attempt >= CONNECT_ATTEMPTS {
                return Err(ClientError::NoConnection);
            }
            Syscall::sleep(CONNECT_RETRY_DELAY_MS);
        }
    }

    pub fn open(&self, name: &str, mode: u8) -> Result<VfsFile, ClientError> {
        let Some(frame) = wire::encode_open(mode, name) else {
            return Err(ClientError::Vfs(VfsError::InvalidPath));
        };
        match rpc(self.connection, &frame, Expect::Opened)? {
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
        let Some(frame) = wire::encode_delete(name) else {
            return Err(ClientError::Vfs(VfsError::InvalidPath));
        };
        match rpc(self.connection, &frame, Expect::Ack)? {
            Reply::Ack => Ok(()),
            Reply::Err(error) => Err(ClientError::Vfs(error)),
            _ => Err(ClientError::Protocol),
        }
    }

    pub fn list(&self) -> Result<Vec<String>, ClientError> {
        let frame = wire::encode_list();
        match rpc(self.connection, &frame, Expect::Listed)? {
            Reply::Eof => Ok(Vec::new()),
            Reply::Listed { token, total: _ } => {
                let blob = drain_data(token)?;
                if blob.is_empty() {
                    return Err(ClientError::Protocol);
                }
                let text = String::from_utf8(blob).map_err(|_| ClientError::Protocol)?;
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
        let mut delivered = 0usize;
        while delivered < buf.len() {
            let want = (buf.len() - delivered).min(wire::RPC_MAX_DATA_BYTES);
            let frame = wire::encode_read(self.handle, self.offset, want as u32);
            match rpc(self.connection, &frame, Expect::ReadData)? {
                Reply::Eof => return Ok(delivered),
                Reply::ReadData { token, count } => {
                    let chunk = drain_data(token)?;
                    let taken = chunk.len().min(count as usize).min(buf.len() - delivered);
                    if taken == 0 {
                        return Err(ClientError::Protocol);
                    }
                    buf[delivered..delivered + taken].copy_from_slice(&chunk[..taken]);
                    delivered += taken;
                    self.offset += taken as u64;
                }
                Reply::Err(error) => return Err(ClientError::Vfs(error)),
                _ => return Err(ClientError::Protocol),
            }
        }
        Ok(delivered)
    }

    pub fn write(&mut self, data: &[u8]) -> Result<(), ClientError> {
        let mut consumed = 0usize;
        loop {
            let end = (consumed + wire::RPC_MAX_DATA_BYTES).min(data.len());
            let chunk = &data[consumed..end];
            let mut object =
                Syscall::ipc_data_write(self.connection, chunk).map_err(ClientError::Data)?;
            let frame = wire::encode_write(self.handle, self.offset, data_token(&object));
            let acknowledged = match rpc(self.connection, &frame, Expect::Written) {
                Ok(Reply::Written { bytes }) => bytes,
                Ok(Reply::Err(error)) => {
                    let _ = object.release();
                    return Err(ClientError::Vfs(error));
                }
                Ok(_) => {
                    let _ = object.release();
                    return Err(ClientError::Protocol);
                }
                Err(error) => {
                    let _ = object.release();
                    return Err(error);
                }
            };
            let _ = object.release();
            let advanced = (acknowledged as usize).min(chunk.len());
            consumed += advanced;
            self.offset += advanced as u64;
            if acknowledged as usize != chunk.len() {
                return Err(ClientError::Protocol);
            }
            if consumed == data.len() {
                return Ok(());
            }
        }
    }

    pub fn seek(&mut self, offset: u64) {
        self.offset = offset;
    }

    pub fn close(self) -> Result<(), ClientError> {
        let frame = wire::encode_close(self.handle);
        match rpc(self.connection, &frame, Expect::Ack)? {
            Reply::Ack => Ok(()),
            Reply::Err(error) => Err(ClientError::Vfs(error)),
            _ => Err(ClientError::Protocol),
        }
    }

    pub fn offset(&self) -> u64 {
        self.offset
    }
}

fn rpc(
    connection: UserConnectionHandle,
    frame: &[u8; 60],
    expect: Expect,
) -> Result<Reply, ClientError> {
    Syscall::ipc_send(connection, frame).map_err(|_| ClientError::Transport)?;
    let Some(future) = Syscall::ipc_receive_message(connection) else {
        return Err(ClientError::Transport);
    };
    let received = Syscall::wait_for_message(future).map_err(|_| ClientError::Transport)?;
    if received.connection().is_none() {
        return Err(ClientError::NoConnection);
    }
    wire::decode_reply(received.data(), expect).map_err(|_| ClientError::Protocol)
}

fn drain_data(token: u32) -> Result<Vec<u8>, ClientError> {
    let mut payload = Vec::new();
    loop {
        let mut chunk = [0u8; DRAIN_CHUNK_BYTES];
        let copied =
            Syscall::ipc_data_read(token as usize, &mut chunk).map_err(ClientError::Data)?;
        if copied == 0 {
            return Ok(payload);
        }
        payload.extend_from_slice(&chunk[..copied]);
    }
}

fn data_token(handle: &UserDataHandle) -> u32 {
    u32::from_str_radix(&format!("{}", handle), 16).expect("canonical data token")
}
