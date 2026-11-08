use crate::client::{transact_frame, ClientError};
use crate::manager::wire::{self, DeviceInfo, DeviceKind, DeviceState, Reply, Request};
use alloc::string::String;
use alloc::vec::Vec;
use core::cmp::min;
use usrlib::handle::UserConnectionHandle;
use usrlib::syscall::Syscall;

const CONNECT_ATTEMPTS: usize = 50;
const CONNECT_RETRY_DELAY_MS: u64 = 100;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Geometry {
    pub block_count: u64,
    pub block_size: u32,
    pub kind: DeviceKind,
    pub state: DeviceState,
    pub description: String,
}

fn transact(
    connection: UserConnectionHandle,
    request: Request<'_>,
) -> Result<[u8; wire::FRAME_BYTES], ClientError> {
    let frame = request.encode().map_err(ClientError::Block)?;
    transact_frame(connection, frame)
}

pub struct Manager {
    connection: UserConnectionHandle,
}

impl Manager {
    pub fn connect() -> Result<Manager, ClientError> {
        for _ in 0..CONNECT_ATTEMPTS {
            if let Ok(connection) = Syscall::ipc_connect(wire::SERVICE_NAME) {
                return Ok(Manager { connection });
            }
            Syscall::sleep(CONNECT_RETRY_DELAY_MS);
        }
        Err(ClientError::NoConnection)
    }

    pub fn register(&self, service: &str) -> Result<(), ClientError> {
        let frame = transact(self.connection, Request::Register { service })?;
        match Reply::decode(&frame).map_err(ClientError::Block)? {
            Reply::Ack => Ok(()),
            Reply::Err(error) => Err(ClientError::Block(error)),
            _ => Err(ClientError::Protocol),
        }
    }

    pub fn list(&self) -> Result<Vec<DeviceInfo>, ClientError> {
        let frame = transact(self.connection, Request::List)?;
        match Reply::decode(&frame).map_err(ClientError::Block)? {
            Reply::Data { token } => {
                let blob = crate::client::drain_data_object(token)?;
                wire::decode_device_list(&blob).map_err(ClientError::Block)
            }
            Reply::Err(error) => Err(ClientError::Block(error)),
            _ => Err(ClientError::Protocol),
        }
    }

    pub fn probe(&self, name: &str) -> Result<Geometry, ClientError> {
        let frame = transact(self.connection, Request::Probe { name })?;
        match Reply::decode(&frame).map_err(ClientError::Block)? {
            Reply::Probe {
                block_count,
                block_size,
                kind,
                state,
                description,
            } => Ok(Geometry {
                block_count,
                block_size,
                kind,
                state,
                description: String::from(description),
            }),
            Reply::Err(error) => Err(ClientError::Block(error)),
            _ => Err(ClientError::Protocol),
        }
    }

    pub fn read_blocks(&self, name: &str, block: u64, blocks: usize) -> Result<Vec<u8>, ClientError> {
        let mut payload = Vec::new();
        let mut requested = 0usize;
        while requested < blocks {
            let chunk_blocks =
                min(blocks - requested, crate::wire::MAX_BLOCKS_PER_REQUEST) as u16;
            let request = Request::Read {
                name,
                block: block + requested as u64,
                count: chunk_blocks,
            };
            let frame = transact(self.connection, request)?;
            match Reply::decode(&frame).map_err(ClientError::Block)? {
                Reply::Data { token } => {
                    let chunk = crate::client::drain_data_object(token)?;
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
        name: &str,
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
            let chunk_blocks = min(blocks - written_blocks, crate::wire::MAX_BLOCKS_PER_REQUEST);
            let chunk_end = min(written_bytes + chunk_blocks * bytes_per_block, data.len());
            let chunk = &data[written_bytes..chunk_end];
            let mut object =
                Syscall::ipc_data_write(self.connection, chunk).map_err(ClientError::Data)?;
            let request = Request::Write {
                name,
                block: block + written_blocks as u64,
                count: chunk_blocks as u16,
                token: crate::client::data_token(&object),
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

    pub fn flush(&self, name: &str) -> Result<(), ClientError> {
        let frame = transact(self.connection, Request::Flush { name })?;
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
