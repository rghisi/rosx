use crate::wire::{
    self, AtaError, Reply, Request, LBA28_MAX, MAX_SECTORS_PER_REQUEST, SECTOR_BYTES,
};
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
    Ata(AtaError),
    Transport,
    NoConnection,
    Data(DataError),
    Protocol,
}

pub struct DriveIdentity {
    pub model: String,
    pub firmware_rev: String,
    pub sectors: u64,
}

fn data_token(handle: &UserDataHandle) -> u32 {
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

fn drain_data_object(token: u32) -> Result<Vec<u8>, ClientError> {
    let drained = drain_chunks(token);
    if let Some(mut object) = UserDataHandle::from_token(token as usize) {
        let _ = object.release();
    }
    drained
}

fn transact_frame(
    connection: UserConnectionHandle,
    request: Request,
) -> Result<[u8; wire::FRAME_BYTES], ClientError> {
    let frame = request.encode().map_err(ClientError::Ata)?;
    Syscall::ipc_send(connection, &frame).map_err(|_| ClientError::Transport)?;
    let future = Syscall::ipc_receive_message(connection).ok_or(ClientError::Transport)?;
    let received = Syscall::wait_for_message(future).map_err(|_| ClientError::Transport)?;
    if received.connection().is_none() {
        return Err(ClientError::NoConnection);
    }
    Ok(*received.data())
}

fn lba28(lba: u64) -> Result<u32, ClientError> {
    if lba > LBA28_MAX as u64 {
        return Err(ClientError::Ata(AtaError::BadAddress));
    }
    Ok(lba as u32)
}

pub struct Ata {
    connection: UserConnectionHandle,
}

impl Ata {
    pub fn connect() -> Result<Ata, ClientError> {
        for _ in 0..CONNECT_ATTEMPTS {
            if let Ok(connection) = Syscall::ipc_connect(wire::SERVICE_NAME) {
                return Ok(Ata { connection });
            }
            Syscall::sleep(CONNECT_RETRY_DELAY_MS);
        }
        Err(ClientError::NoConnection)
    }

    pub fn identify(&self, drive: u8) -> Result<DriveIdentity, ClientError> {
        let frame = transact_frame(self.connection, Request::Identify { drive })?;
        match Reply::decode(&frame).map_err(ClientError::Ata)? {
            Reply::Ident {
                sectors,
                firmware_rev,
                model,
            } => Ok(DriveIdentity {
                model: String::from(model),
                firmware_rev: String::from(firmware_rev),
                sectors,
            }),
            Reply::Err { error, .. } => Err(ClientError::Ata(error)),
            _ => Err(ClientError::Protocol),
        }
    }

    pub fn read(&self, drive: u8, lba: u64, sector_count: usize) -> Result<Vec<u8>, ClientError> {
        let mut payload = Vec::with_capacity(sector_count * SECTOR_BYTES);
        while payload.len() < sector_count * SECTOR_BYTES {
            let sectors_done = payload.len() / SECTOR_BYTES;
            let remaining = sector_count - sectors_done;
            let chunk_sectors = min(remaining, MAX_SECTORS_PER_REQUEST) as u8;
            let request = Request::Read {
                drive,
                lba: lba28(lba + sectors_done as u64)?,
                count: chunk_sectors,
            };
            let frame = transact_frame(self.connection, request)?;
            match Reply::decode(&frame).map_err(ClientError::Ata)? {
                Reply::Data { token, sectors } => {
                    let chunk = drain_data_object(token)?;
                    if chunk.len() != sectors as usize * SECTOR_BYTES {
                        return Err(ClientError::Protocol);
                    }
                    payload.extend_from_slice(&chunk);
                }
                Reply::Err { error, .. } => return Err(ClientError::Ata(error)),
                _ => return Err(ClientError::Protocol),
            }
        }
        Ok(payload)
    }

    pub fn write(&self, drive: u8, lba: u64, data: &[u8]) -> Result<(), ClientError> {
        if data.len() % SECTOR_BYTES != 0 {
            return Err(ClientError::Protocol);
        }
        let mut written = 0usize;
        while written < data.len() {
            let sectors_done = written / SECTOR_BYTES;
            let chunk_end = min(
                written + MAX_SECTORS_PER_REQUEST * SECTOR_BYTES,
                data.len(),
            );
            let chunk = &data[written..chunk_end];
            let mut object =
                Syscall::ipc_data_write(self.connection, chunk).map_err(ClientError::Data)?;
            let request = Request::Write {
                drive,
                lba: lba28(lba + sectors_done as u64)?,
                count: (chunk.len() / SECTOR_BYTES) as u8,
                token: data_token(&object),
            };
            let reply = transact_frame(self.connection, request);
            let _ = object.release();
            match Reply::decode(&reply?).map_err(ClientError::Ata)? {
                Reply::Ack => written = chunk_end,
                Reply::Err { error, .. } => return Err(ClientError::Ata(error)),
                _ => return Err(ClientError::Protocol),
            }
        }
        Ok(())
    }

    pub fn flush(&self, drive: u8) -> Result<(), ClientError> {
        let frame = transact_frame(self.connection, Request::Flush { drive })?;
        match Reply::decode(&frame).map_err(ClientError::Ata)? {
            Reply::Ack => Ok(()),
            Reply::Err { error, .. } => Err(ClientError::Ata(error)),
            _ => Err(ClientError::Protocol),
        }
    }

    pub fn disconnect(self) {
        Syscall::ipc_disconnect(self.connection);
    }
}
