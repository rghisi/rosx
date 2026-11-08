use crate::wire::{self, BlockError, Reply, Request, MAX_BLOCKS_PER_REQUEST};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::cmp::min;
use usrlib::error::DataError;
use usrlib::handle::{UserConnectionHandle, UserDataHandle};
use usrlib::syscall::Syscall;

const CONNECT_ATTEMPTS: usize = 50;
const CONNECT_RETRY_DELAY_MS: u64 = 100;
const DATA_READ_CHUNK_BYTES: usize = 4096;

#[derive(Debug)]
pub enum ClientError {
    Block(BlockError),
    Transport,
    NoConnection,
    Data(DataError),
    Protocol,
}

pub struct Geometry {
    pub block_count: u64,
    pub block_size: u32,
    pub description: String,
}

pub(crate) fn data_token(handle: &UserDataHandle) -> u32 {
    u32::from_str_radix(&format!("{}", handle), 16).expect("canonical data token")
}

fn drain_chunks(token: u32) -> Result<Vec<u8>, ClientError> {
    let mut payload = Vec::new();
    loop {
        let mut chunk = [0u8; DATA_READ_CHUNK_BYTES];
        let copied =
            Syscall::ipc_data_read(token as usize, &mut chunk).map_err(ClientError::Data)?;
        if copied == 0 {
            return Ok(payload);
        }
        payload.extend_from_slice(&chunk[..copied]);
    }
}

pub(crate) fn drain_data_object(token: u32) -> Result<Vec<u8>, ClientError> {
    let drained = drain_chunks(token);
    if let Some(mut object) = UserDataHandle::from_token(token as usize) {
        let _ = object.release();
    }
    drained
}

pub(crate) fn transact_frame(
    connection: UserConnectionHandle,
    frame: [u8; wire::FRAME_BYTES],
) -> Result<[u8; wire::FRAME_BYTES], ClientError> {
    Syscall::ipc_send(connection, &frame).map_err(|_| ClientError::Transport)?;
    let future = Syscall::ipc_receive_message(connection).ok_or(ClientError::Transport)?;
    let received = Syscall::wait_for_message(future).map_err(|_| ClientError::Transport)?;
    if received.connection().is_none() {
        return Err(ClientError::NoConnection);
    }
    Ok(*received.data())
}

fn transact(
    connection: UserConnectionHandle,
    request: Request,
) -> Result<[u8; wire::FRAME_BYTES], ClientError> {
    let frame = request.encode().map_err(ClientError::Block)?;
    transact_frame(connection, frame)
}

pub struct Block {
    connection: UserConnectionHandle,
}

impl Block {
    pub fn connect(service: &str) -> Result<Block, ClientError> {
        for _ in 0..CONNECT_ATTEMPTS {
            if let Ok(connection) = Syscall::ipc_connect(service) {
                return Ok(Block { connection });
            }
            Syscall::sleep(CONNECT_RETRY_DELAY_MS);
        }
        Err(ClientError::NoConnection)
    }

    pub fn probe(&self, device: u32) -> Result<Geometry, ClientError> {
        let frame = transact(self.connection, Request::Probe { device })?;
        match Reply::decode(&frame).map_err(ClientError::Block)? {
            Reply::Probe {
                block_count,
                block_size,
                description,
            } => Ok(Geometry {
                block_count,
                block_size,
                description: String::from(description),
            }),
            Reply::Err(error) => Err(ClientError::Block(error)),
            _ => Err(ClientError::Protocol),
        }
    }

    pub fn read_blocks(&self, device: u32, block: u64, blocks: usize) -> Result<Vec<u8>, ClientError> {
        let mut payload = Vec::new();
        let mut requested = 0usize;
        while requested < blocks {
            let chunk_blocks = min(blocks - requested, MAX_BLOCKS_PER_REQUEST) as u16;
            let request = Request::Read {
                device,
                block: block + requested as u64,
                count: chunk_blocks,
            };
            let frame = transact(self.connection, request)?;
            match Reply::decode(&frame).map_err(ClientError::Block)? {
                Reply::Data { token } => {
                    let chunk = drain_data_object(token)?;
                    if chunk.is_empty() {
                        return Err(ClientError::Protocol);
                    }
                    payload.extend_from_slice(&chunk);
                }
                Reply::Err(error) => return Err(ClientError::Block(error)),
                _ => return Err(ClientError::Protocol),
            }
            requested += chunk_blocks as usize;
        }
        Ok(payload)
    }

    pub fn write_blocks(
        &self,
        device: u32,
        block: u64,
        blocks: usize,
        data: &[u8],
    ) -> Result<(), ClientError> {
        if blocks == 0 || data.is_empty() || data.len() % blocks != 0 {
            return Err(ClientError::Protocol);
        }
        let bytes_per_block = data.len() / blocks;
        let mut written_blocks = 0usize;
        let mut written_bytes = 0usize;
        while written_blocks < blocks {
            let chunk_blocks = min(blocks - written_blocks, MAX_BLOCKS_PER_REQUEST);
            let chunk_end = min(written_bytes + chunk_blocks * bytes_per_block, data.len());
            let chunk = &data[written_bytes..chunk_end];
            let mut object =
                Syscall::ipc_data_write(self.connection, chunk).map_err(ClientError::Data)?;
            let request = Request::Write {
                device,
                block: block + written_blocks as u64,
                count: chunk_blocks as u16,
                token: data_token(&object),
            };
            let encoded = request.encode().map_err(ClientError::Block)?;
            let reply = transact_frame(self.connection, encoded);
            let _ = object.release();
            match Reply::decode(&reply?).map_err(ClientError::Block)? {
                Reply::Ack => {
                    written_blocks += chunk_blocks;
                    written_bytes = chunk_end;
                }
                Reply::Err(error) => return Err(ClientError::Block(error)),
                _ => return Err(ClientError::Protocol),
            }
        }
        Ok(())
    }

    pub fn flush(&self, device: u32) -> Result<(), ClientError> {
        let frame = transact(self.connection, Request::Flush { device })?;
        match Reply::decode(&frame).map_err(ClientError::Block)? {
            Reply::Ack => Ok(()),
            Reply::Err(error) => Err(ClientError::Block(error)),
            _ => Err(ClientError::Protocol),
        }
    }

    pub fn disconnect(self) {
        Syscall::ipc_disconnect(self.connection);
    }
}
