#![no_std]

extern crate alloc;

use alloc::boxed::Box;
use alloc::format;
use alloc::vec::Vec;
use block_protocol::manager::wire::{self, Reply, Request};
use block_protocol::wire::BlockError;
use blockd::namespace::DeviceTable;
use blockd::provider::{AtaProvider, Provider, ATA_SERVICE};
use usrlib::handle::{UserBindingHandle, UserConnectionHandle, UserDataHandle};
use usrlib::println;
use usrlib::syscall::Syscall;

const REBIND_DELAY_MS: u64 = 100;
const INCOMING_READ_CHUNK_BYTES: usize = 4096;

fn conn_token(connection: UserConnectionHandle) -> u32 {
    u32::from_str_radix(&format!("{}", connection), 16).expect("canonical connection token")
}

fn data_token(handle: &UserDataHandle) -> u32 {
    u32::from_str_radix(&format!("{}", handle), 16).expect("canonical data token")
}

fn drain_incoming(token: u32) -> Option<Vec<u8>> {
    let mut payload = Vec::new();
    loop {
        let mut chunk = [0u8; INCOMING_READ_CHUNK_BYTES];
        let copied = Syscall::ipc_data_read(token as usize, &mut chunk).ok()?;
        if copied == 0 {
            return Some(payload);
        }
        payload.extend_from_slice(&chunk[..copied]);
    }
}

fn open_provider(service: &str) -> Result<Box<dyn Provider>, BlockError> {
    if blockd::namespace::provider_source(service).is_none() {
        return Err(BlockError::Unsupported);
    }
    AtaProvider::dial(ATA_SERVICE).map(|provider| Box::new(provider) as Box<dyn Provider>)
}

fn main() {
    println!("[BLOCKD] block device manager");
    let mut table = DeviceTable::new();
    let mut held: Vec<(u32, UserDataHandle)> = Vec::new();
    loop {
        match Syscall::ipc_bind(wire::SERVICE_NAME) {
            Ok(binding) => {
                println!("[BLOCKD] ready");
                serve(&mut table, &mut held, binding);
            }
            Err(_) => Syscall::sleep(REBIND_DELAY_MS),
        }
    }
}

fn serve(table: &mut DeviceTable, held: &mut Vec<(u32, UserDataHandle)>, binding: UserBindingHandle) {
    loop {
        if let Some(future) = Syscall::ipc_accept_message(binding) {
            if let Ok(received) = Syscall::wait_for_message(future) {
                serve_one(table, held, &received);
            }
        }
    }
}

fn serve_one(
    table: &mut DeviceTable,
    held: &mut Vec<(u32, UserDataHandle)>,
    received: &usrlib::ipc::ReceivedMessage,
) {
    let Some(connection) = received.connection() else {
        return;
    };
    let owner = conn_token(connection);
    release_held_for(held, owner);
    let reply = match Request::decode(received.data()) {
        Ok(request) => dispatch(table, held, connection, owner, request),
        Err(error) => encode_reply(&Reply::Err(error)),
    };
    let _ = Syscall::ipc_send(connection, &reply);
}

fn release_held_for(held: &mut Vec<(u32, UserDataHandle)>, owner: u32) {
    held.retain_mut(|(held_owner, object)| {
        if *held_owner == owner {
            let _ = object.release();
            false
        } else {
            true
        }
    });
}

fn dispatch(
    table: &mut DeviceTable,
    held: &mut Vec<(u32, UserDataHandle)>,
    connection: UserConnectionHandle,
    owner: u32,
    request: Request<'_>,
) -> [u8; wire::FRAME_BYTES] {
    match request {
        Request::Register { service } => register(table, service),
        Request::List => match wire::encode_device_list(&table.list()) {
            Err(error) => encode_reply(&Reply::Err(error)),
            Ok(blob) => payload_reply(held, connection, owner, &blob),
        },
        Request::Probe { name } => match table.probe(name) {
            Err(error) => encode_reply(&Reply::Err(error)),
            Ok(device) => encode_reply(&Reply::Probe {
                block_count: device.block_count,
                block_size: device.block_size,
                kind: device.kind,
                state: device.state,
                description: &device.description,
            }),
        },
        Request::Read { name, block, count } => match table.read(name, block, count) {
            Err(error) => encode_reply(&Reply::Err(error)),
            Ok(payload) => payload_reply(held, connection, owner, &payload),
        },
        Request::Write { name, block, count, token } => match drain_incoming(token) {
            None => encode_reply(&Reply::Err(BlockError::Protocol)),
            Some(payload) => match table.write(name, block, count, &payload) {
                Err(error) => encode_reply(&Reply::Err(error)),
                Ok(()) => encode_reply(&Reply::Ack),
            },
        },
        Request::Flush { name } => match table.flush(name) {
            Err(error) => encode_reply(&Reply::Err(error)),
            Ok(()) => encode_reply(&Reply::Ack),
        },
    }
}

fn register(table: &mut DeviceTable, service: &str) -> [u8; wire::FRAME_BYTES] {
    let provider = match open_provider(service) {
        Err(error) => return encode_reply(&Reply::Err(error)),
        Ok(provider) => provider,
    };
    match table.register(service, provider) {
        Err(error) => encode_reply(&Reply::Err(error)),
        Ok(verdicts) => {
            for verdict in verdicts {
                println!("[BLOCKD] {verdict}");
            }
            encode_reply(&Reply::Ack)
        }
    }
}

fn payload_reply(
    held: &mut Vec<(u32, UserDataHandle)>,
    connection: UserConnectionHandle,
    owner: u32,
    payload: &[u8],
) -> [u8; wire::FRAME_BYTES] {
    match Syscall::ipc_data_write(connection, payload) {
        Err(_) => encode_reply(&Reply::Err(BlockError::Busy)),
        Ok(object) => {
            let token = data_token(&object);
            held.push((owner, object));
            encode_reply(&Reply::Data { token })
        }
    }
}

fn encode_reply(reply: &Reply<'_>) -> [u8; wire::FRAME_BYTES] {
    reply.encode().unwrap_or_else(|_| protocol_error())
}

fn protocol_error() -> [u8; wire::FRAME_BYTES] {
    Reply::Err(BlockError::Protocol)
        .encode()
        .expect("protocol error reply always encodes")
}
