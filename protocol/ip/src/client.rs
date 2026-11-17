use crate::wire::{self, IPV4_BYTES, IpError, Neighbour, Reply, Request, Status};
use alloc::format;
use alloc::vec::Vec;
use usrlib::error::DataError;
use usrlib::handle::{UserConnectionHandle, UserDataHandle, UserFutureHandle};
use usrlib::syscall::Syscall;

const CONNECT_ATTEMPTS: usize = 50;
const CONNECT_RETRY_DELAY_MS: u64 = 100;
const DATA_READ_CHUNK_BYTES: usize = 4096;

#[derive(Debug)]
pub enum ClientError {
    Ip(IpError),
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
        let copied =
            Syscall::ipc_data_read(token as usize, &mut chunk).map_err(ClientError::Data)?;
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IpEvent {
    Datagram {
        source: [u8; IPV4_BYTES],
        protocol: u8,
        payload: Vec<u8>,
    },
    Unreachable {
        destination: [u8; IPV4_BYTES],
        reason: wire::UnreachableReason,
    },
}

pub struct Ip {
    connection: UserConnectionHandle,
    outbound: Vec<UserDataHandle>,
}

impl Ip {
    pub fn connect() -> Result<Ip, ClientError> {
        for _ in 0..CONNECT_ATTEMPTS {
            if let Ok(connection) = Syscall::ipc_connect(wire::SERVICE_NAME) {
                return Ok(Ip {
                    connection,
                    outbound: Vec::new(),
                });
            }
            Syscall::sleep(CONNECT_RETRY_DELAY_MS);
        }
        Err(ClientError::NoConnection)
    }

    pub fn connection(&self) -> UserConnectionHandle {
        self.connection
    }

    pub fn status(&mut self) -> Result<Status, ClientError> {
        let frame = self.transact(Request::Status)?;
        match Reply::decode(&frame).map_err(ClientError::Ip)? {
            Reply::Status(status) => Ok(status),
            Reply::Err { error } => Err(ClientError::Ip(error)),
            _ => Err(ClientError::Protocol),
        }
    }

    pub fn configure(
        &mut self,
        address: [u8; IPV4_BYTES],
        prefix_len: u8,
        gateway: Option<[u8; IPV4_BYTES]>,
    ) -> Result<(), ClientError> {
        let frame = self.transact(Request::Config {
            address,
            prefix_len,
            gateway,
        })?;
        match Reply::decode(&frame).map_err(ClientError::Ip)? {
            Reply::Ack => Ok(()),
            Reply::Err { error } => Err(ClientError::Ip(error)),
            _ => Err(ClientError::Protocol),
        }
    }

    pub fn neighbours(&mut self) -> Result<Vec<Neighbour>, ClientError> {
        let frame = self.transact(Request::Neigh)?;
        match Reply::decode(&frame).map_err(ClientError::Ip)? {
            Reply::Neigh { token, entries } => {
                let blob = drain_data_object(token)?;
                let neighbours = wire::decode_neighbours_table(&blob).map_err(ClientError::Ip)?;
                if neighbours.len() != entries as usize {
                    return Err(ClientError::Protocol);
                }
                Ok(neighbours)
            }
            Reply::Err { error } => Err(ClientError::Ip(error)),
            _ => Err(ClientError::Protocol),
        }
    }

    pub fn subscribe(&mut self, icmp_kind: u8) -> Result<(), ClientError> {
        let frame = self.transact(Request::Subscribe { icmp_kind })?;
        match Reply::decode(&frame).map_err(ClientError::Ip)? {
            Reply::Ack => Ok(()),
            Reply::Err { error } => Err(ClientError::Ip(error)),
            _ => Err(ClientError::Protocol),
        }
    }

    pub fn send_ip(
        &mut self,
        destination: [u8; IPV4_BYTES],
        protocol: u8,
        payload: &[u8],
    ) -> Result<(), ClientError> {
        if payload.is_empty() || payload.len() > wire::MAX_IP_PAYLOAD_BYTES {
            return Err(ClientError::Ip(IpError::Oversize));
        }
        let object =
            Syscall::ipc_data_write(self.connection, payload).map_err(ClientError::Data)?;
        let frame = Request::SendIp {
            destination,
            protocol,
            token: data_token(&object),
            length: payload.len() as u16,
        }
        .encode()
        .map_err(ClientError::Ip)?;
        Syscall::ipc_send(self.connection, &frame).map_err(|_| ClientError::Transport)?;
        self.outbound.push(object);
        Ok(())
    }

    pub fn receive_future(&self) -> Result<UserFutureHandle, ClientError> {
        Syscall::ipc_receive_message(self.connection).ok_or(ClientError::Transport)
    }

    pub fn consume_receive(&mut self, future: UserFutureHandle) -> Result<IpEvent, ClientError> {
        let received = Syscall::wait_for_message(future).map_err(|_| ClientError::Transport)?;
        if received.connection().is_none() {
            return Err(ClientError::NoConnection);
        }
        self.release_outbound();
        dispatch_push(received.data())
    }

    pub fn receive(&mut self) -> Result<IpEvent, ClientError> {
        let future = self.receive_future()?;
        self.consume_receive(future)
    }

    pub fn disconnect(mut self) {
        for object in self.outbound.iter_mut() {
            let _ = object.release();
        }
        Syscall::ipc_disconnect(self.connection);
    }

    fn transact(&mut self, request: Request) -> Result<[u8; wire::FRAME_BYTES], ClientError> {
        let frame = request.encode().map_err(ClientError::Ip)?;
        Syscall::ipc_send(self.connection, &frame).map_err(|_| ClientError::Transport)?;
        let future = Syscall::ipc_receive_message(self.connection).ok_or(ClientError::Transport)?;
        let received = Syscall::wait_for_message(future).map_err(|_| ClientError::Transport)?;
        if received.connection().is_none() {
            return Err(ClientError::NoConnection);
        }
        self.release_outbound();
        Ok(*received.data())
    }

    fn release_outbound(&mut self) {
        for object in self.outbound.iter_mut() {
            let _ = object.release();
        }
        self.outbound.clear();
    }
}

fn dispatch_push(frame: &[u8; wire::FRAME_BYTES]) -> Result<IpEvent, ClientError> {
    match Reply::decode(frame).map_err(ClientError::Ip)? {
        Reply::PushDatagram {
            source,
            protocol,
            token,
            length,
        } => {
            let payload = drain_data_object(token)?;
            if payload.len() != length as usize {
                return Err(ClientError::Protocol);
            }
            Ok(IpEvent::Datagram {
                source,
                protocol,
                payload,
            })
        }
        Reply::PushUnreachable {
            destination,
            reason,
        } => Ok(IpEvent::Unreachable {
            destination,
            reason,
        }),
        Reply::Err { error } => Err(ClientError::Ip(error)),
        _ => Err(ClientError::Protocol),
    }
}
