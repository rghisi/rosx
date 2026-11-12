use crate::wire::{self, PciError, Reply, Request, Slot};
use alloc::string::String;
use alloc::vec::Vec;
use usrlib::error::DataError;
use usrlib::handle::{UserConnectionHandle, UserDataHandle};
use usrlib::syscall::Syscall;

const CONNECT_ATTEMPTS: usize = 50;
const CONNECT_RETRY_DELAY_MS: u64 = 100;
const DATA_READ_CHUNK_BYTES: usize = 4096;

#[derive(Debug)]
pub enum ClientError {
    Pci(PciError),
    Transport,
    NoConnection,
    NoDevice(String),
    Data(DataError),
    Protocol,
}

fn transact_frame(
    connection: UserConnectionHandle,
    request: Request,
) -> Result<[u8; wire::FRAME_BYTES], ClientError> {
    let frame = request.encode().map_err(ClientError::Pci)?;
    Syscall::ipc_send(connection, &frame).map_err(|_| ClientError::Transport)?;
    let future = Syscall::ipc_receive_message(connection).ok_or(ClientError::Transport)?;
    let received = Syscall::wait_for_message(future).map_err(|_| ClientError::Transport)?;
    if received.connection().is_none() {
        return Err(ClientError::NoConnection);
    }
    Ok(*received.data())
}

fn drain_slots(token: u32) -> Result<Vec<u8>, ClientError> {
    let mut blob = Vec::new();
    loop {
        let mut chunk = [0u8; DATA_READ_CHUNK_BYTES];
        let copied = Syscall::ipc_data_read(token as usize, &mut chunk).map_err(ClientError::Data)?;
        if copied == 0 {
            break;
        }
        blob.extend_from_slice(&chunk[..copied]);
    }
    if let Some(mut object) = UserDataHandle::from_token(token as usize) {
        let _ = object.release();
    }
    Ok(blob)
}

pub struct Pci {
    connection: UserConnectionHandle,
}

impl Pci {
    pub fn connect() -> Result<Pci, ClientError> {
        for _ in 0..CONNECT_ATTEMPTS {
            if let Ok(connection) = Syscall::ipc_connect(wire::SERVICE_NAME) {
                return Ok(Pci { connection });
            }
            Syscall::sleep(CONNECT_RETRY_DELAY_MS);
        }
        Err(ClientError::NoConnection)
    }

    pub fn find(&self, vendor_id: u16, device_id: u16) -> Result<Option<Slot>, ClientError> {
        let frame = transact_frame(self.connection, Request::Find { vendor_id, device_id })?;
        match Reply::decode(&frame).map_err(ClientError::Pci)? {
            Reply::Slot(slot) => Ok(Some(slot)),
            Reply::Missing => Ok(None),
            Reply::Err { error } => Err(ClientError::Pci(error)),
            _ => Err(ClientError::Protocol),
        }
    }

    pub fn enable(&self, slot: &Slot) -> Result<Slot, ClientError> {
        let request = Request::Enable {
            bus: slot.bus,
            device: slot.device,
            function: slot.function,
        };
        let frame = transact_frame(self.connection, request)?;
        match Reply::decode(&frame).map_err(ClientError::Pci)? {
            Reply::Slot(updated) => Ok(updated),
            Reply::Missing => Err(ClientError::NoDevice(slot.location().unwrap_or_default())),
            Reply::Err { error } => Err(ClientError::Pci(error)),
            _ => Err(ClientError::Protocol),
        }
    }

    pub fn list(&self, bus: u8) -> Result<Vec<Slot>, ClientError> {
        let frame = transact_frame(self.connection, Request::List { bus })?;
        match Reply::decode(&frame).map_err(ClientError::Pci)? {
            Reply::List { token, entries } => {
                let slots =
                    wire::decode_slots_table(&drain_slots(token)?).map_err(ClientError::Pci)?;
                if slots.len() != entries as usize {
                    return Err(ClientError::Protocol);
                }
                Ok(slots)
            }
            Reply::Err { error } => Err(ClientError::Pci(error)),
            _ => Err(ClientError::Protocol),
        }
    }

    pub fn disconnect(self) {
        Syscall::ipc_disconnect(self.connection);
    }
}
