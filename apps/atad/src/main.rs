#![no_std]

extern crate alloc;

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use ata_protocol::wire::{self, AtaError, Reply, Request, LBA28_MAX, SECTOR_BYTES};
use atad::taskfile::{AtaDrive, Bus, DriveFault, Identity};
use block_protocol::wire as block_wire;
use usrlib::error::PortError;
use usrlib::handle::{UserBindingHandle, UserConnectionHandle, UserDataHandle};
use usrlib::println;
use usrlib::syscall::Syscall;

const PRIMARY_CHANNEL: &str = "ata0";
const PRIMARY_TASKFILE_BASE: u16 = 0x1F0;
const PRIMARY_TASKFILE_END: u16 = 0x1F7;
const ALTERNATE_STATUS_PORT: u16 = 0x3F6;
const REBIND_DELAY_MS: u64 = 100;
const INCOMING_READ_CHUNK_BYTES: usize = 4096;
const EXIT_ACQUIRE_REFUSED: u32 = 1;

struct SyscallBus;

impl Bus for SyscallBus {
    fn read_byte(&mut self, port: u16) -> Result<u8, PortError> {
        Syscall::port_in(port, 1).map(|value| value as u8)
    }

    fn write_byte(&mut self, port: u16, value: u8) -> Result<(), PortError> {
        Syscall::port_out(port, value as usize, 1)
    }

    fn read_words(&mut self, port: u16, destination: &mut [u16]) -> Result<(), PortError> {
        Syscall::port_in_words(port, destination)
    }

    fn write_words(&mut self, port: u16, source: &[u16]) -> Result<(), PortError> {
        Syscall::port_out_words(port, source)
    }

    fn sleep_ms(&mut self, millis: u64) {
        Syscall::sleep(millis);
    }
}

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

fn probe_drives() -> [AtaDrive<SyscallBus>; 2] {
    let mut drives = [AtaDrive::new(SyscallBus, 0), AtaDrive::new(SyscallBus, 1)];
    for (slot, drive) in drives.iter_mut().enumerate() {
        if let Ok(identity) = drive.probe() {
            report_ready(slot, &identity);
        }
    }
    drives
}

fn report_ready(slot: usize, identity: &Identity) {
    println!(
        "[ATAD] {} drive{} ready: {} sectors={}",
        PRIMARY_CHANNEL, slot, identity.model, identity.sectors
    );
}

fn acquire_ports() -> bool {
    if let Err(error) = Syscall::port_acquire(PRIMARY_TASKFILE_BASE, PRIMARY_TASKFILE_END) {
        println!("[ATAD] primary taskfile ports refused: {error}");
        return false;
    }
    if let Err(error) = Syscall::port_acquire(ALTERNATE_STATUS_PORT, ALTERNATE_STATUS_PORT) {
        println!("[ATAD] alternate status port refused: {error}");
        let _ = Syscall::port_release(PRIMARY_TASKFILE_BASE, PRIMARY_TASKFILE_END);
        return false;
    }
    true
}

fn main() {
    println!("[ATAD] ATA disk driver");
    if !acquire_ports() {
        Syscall::exit(EXIT_ACQUIRE_REFUSED);
    }
    let mut drives = probe_drives();
    let mut held: Vec<(u32, UserDataHandle)> = Vec::new();
    loop {
        match Syscall::ipc_bind(wire::SERVICE_NAME) {
            Ok(binding) => serve(&mut drives, &mut held, binding),
            Err(_) => Syscall::sleep(REBIND_DELAY_MS),
        }
    }
}

fn serve(drives: &mut [AtaDrive<SyscallBus>], held: &mut Vec<(u32, UserDataHandle)>, binding: UserBindingHandle) {
    loop {
        if let Some(future) = Syscall::ipc_accept_message(binding) {
            if let Ok(received) = Syscall::wait_for_message(future) {
                serve_one(drives, held, &received);
            }
        }
    }
}

fn serve_one(
    drives: &mut [AtaDrive<SyscallBus>],
    held: &mut Vec<(u32, UserDataHandle)>,
    received: &usrlib::ipc::ReceivedMessage,
) {
    let Some(connection) = received.connection() else {
        return;
    };
    let owner = conn_token(connection);
    release_held_for(held, owner);
    if block_wire::is_block_request(received.data()[0]) {
        serve_block_family(drives, held, connection, owner, received.data());
        return;
    }
    let reply = match Request::decode(received.data()) {
        Ok(request) => dispatch(drives, held, connection, owner, request),
        Err(error) => error_reply(error, 0, 0),
    };
    if let Ok(frame) = reply.encode() {
        let _ = Syscall::ipc_send(connection, &frame);
    }
}

fn serve_block_family(
    drives: &mut [AtaDrive<SyscallBus>],
    held: &mut Vec<(u32, UserDataHandle)>,
    connection: UserConnectionHandle,
    owner: u32,
    frame: &[u8; ata_protocol::wire::FRAME_BYTES],
) {
    let reply = match block_wire::Request::decode(frame) {
        Ok(request) => dispatch_block(drives, held, connection, owner, request),
        Err(error) => encode_block_reply(&block_wire::Reply::Err(error)),
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

fn dispatch<'a>(
    drives: &'a mut [AtaDrive<SyscallBus>],
    held: &mut Vec<(u32, UserDataHandle)>,
    connection: UserConnectionHandle,
    owner: u32,
    request: Request,
) -> Reply<'a> {
    match request {
        Request::Identify { drive } => {
            let identity = drive_of(drives, drive).and_then(|drive| drive.identity());
            match identity {
                Some(identity) => Reply::Ident {
                    sectors: identity.sectors,
                    firmware_rev: &identity.firmware_rev,
                    model: &identity.model,
                },
                None => error_reply(AtaError::NotFound, 0, 0),
            }
        }
        Request::Read { drive, lba, count } => {
            read_payload(drives, drive, lba, count).map_or_else(
                |fault| {
                    let (error, error_register, status) = fault.wire_error();
                    error_reply(error, error_register, status)
                },
                |payload| attach_payload(held, connection, owner, &payload, count as u16),
            )
        }
        Request::Write { drive, lba, count, token } => {
            match drain_incoming(token) {
                None => error_reply(AtaError::Protocol, 0, 0),
                Some(payload) if payload.len() != count as usize * SECTOR_BYTES => {
                    error_reply(AtaError::Protocol, 0, 0)
                }
                Some(payload) => match drive_of(drives, drive) {
                    None => error_reply(AtaError::NotFound, 0, 0),
                    Some(engine) => engine
                        .write_sectors(lba, &payload)
                        .map(|()| Reply::Ack)
                        .unwrap_or_else(|fault| {
                            let (error, error_register, status) = fault.wire_error();
                            error_reply(error, error_register, status)
                        }),
                },
            }
        }
        Request::Flush { drive } => match drive_of(drives, drive) {
            None => error_reply(AtaError::NotFound, 0, 0),
            Some(engine) => engine
                .flush()
                .map(|()| Reply::Ack)
                .unwrap_or_else(|fault| {
                    let (error, error_register, status) = fault.wire_error();
                    error_reply(error, error_register, status)
                }),
        },
    }
}

fn read_payload(
    drives: &mut [AtaDrive<SyscallBus>],
    drive: u8,
    lba: u32,
    count: u8,
) -> Result<Vec<u8>, atad::taskfile::DriveFault> {
    let engine = drive_of(drives, drive).ok_or(atad::taskfile::DriveFault::NotFound)?;
    let mut buffer = vec![0u8; count as usize * SECTOR_BYTES];
    engine.read_sectors(lba, count, &mut buffer)?;
    Ok(buffer)
}

fn attach_payload(
    held: &mut Vec<(u32, UserDataHandle)>,
    connection: UserConnectionHandle,
    owner: u32,
    payload: &[u8],
    sectors: u16,
) -> Reply<'static> {
    match Syscall::ipc_data_write(connection, payload) {
        Err(_) => error_reply(AtaError::Busy, 0, 0),
        Ok(object) => {
            let token = data_token(&object);
            held.push((owner, object));
            Reply::Data { token, sectors }
        }
    }
}

fn drive_of(
    drives: &mut [AtaDrive<SyscallBus>],
    drive: u8,
) -> Option<&mut AtaDrive<SyscallBus>> {
    drives.get_mut(drive as usize)
}

fn dispatch_block(
    drives: &mut [AtaDrive<SyscallBus>],
    held: &mut Vec<(u32, UserDataHandle)>,
    connection: UserConnectionHandle,
    owner: u32,
    request: block_wire::Request,
) -> [u8; block_wire::FRAME_BYTES] {
    match request {
        block_wire::Request::Probe { device } => {
            let Some(drive) = block_drive_of(drives, device) else {
                return encode_block_reply(&block_wire::Reply::Err(block_wire::BlockError::NotFound));
            };
            match drive.identity() {
                Some(identity) => encode_block_reply(&block_wire::Reply::Probe {
                    block_count: identity.sectors,
                    block_size: SECTOR_BYTES as u32,
                    description: &block_description(identity),
                }),
                None => encode_block_reply(&block_wire::Reply::Err(block_wire::BlockError::NotFound)),
            }
        }
        block_wire::Request::Read { device, block, count } => {
            match read_block_payload(drives, device, block, count) {
                Err(fault) => encode_block_reply(&block_wire::Reply::Err(block_error(&fault))),
                Ok(payload) => match attach_block_payload(held, connection, owner, &payload) {
                    Ok(reply) => encode_block_reply(&reply),
                    Err(error) => encode_block_reply(&block_wire::Reply::Err(error)),
                },
            }
        }
        block_wire::Request::Write {
            device,
            block,
            count,
            token,
        } => match write_block_payload(drives, device, block, count, token) {
            Ok(()) => encode_block_reply(&block_wire::Reply::Ack),
            Err(error) => encode_block_reply(&block_wire::Reply::Err(error)),
        },
        block_wire::Request::Flush { device } => match block_drive_of(drives, device) {
            None => encode_block_reply(&block_wire::Reply::Err(block_wire::BlockError::NotFound)),
            Some(engine) => match engine.flush() {
                Ok(()) => encode_block_reply(&block_wire::Reply::Ack),
                Err(fault) => encode_block_reply(&block_wire::Reply::Err(block_error(&fault))),
            },
        },
    }
}

fn block_drive_of(
    drives: &mut [AtaDrive<SyscallBus>],
    device: u32,
) -> Option<&mut AtaDrive<SyscallBus>> {
    u8::try_from(device)
        .ok()
        .and_then(|drive| drive_of(drives, drive))
}

fn block_description(identity: &Identity) -> String {
    let description = format!("{} {}", identity.model, identity.firmware_rev);
    let mut bounded = String::new();
    for character in description.chars() {
        if bounded.len() + character.len_utf8() > block_wire::DESCRIPTION_BYTES {
            break;
        }
        bounded.push(character);
    }
    bounded
}

fn block_error(fault: &DriveFault) -> block_wire::BlockError {
    match fault {
        DriveFault::NotFound => block_wire::BlockError::NotFound,
        DriveFault::BadAddress => block_wire::BlockError::BadAddress,
        DriveFault::Busy => block_wire::BlockError::Busy,
        DriveFault::Denied => block_wire::BlockError::Busy,
        DriveFault::Device { .. } => block_wire::BlockError::Device,
    }
}

fn block_lba(block: u64, count: u16) -> Result<u32, block_wire::BlockError> {
    let last = block
        .checked_add(count as u64)
        .ok_or(block_wire::BlockError::BadAddress)?;
    if last > LBA28_MAX as u64 + 1 {
        return Err(block_wire::BlockError::BadAddress);
    }
    u32::try_from(block).map_err(|_| block_wire::BlockError::BadAddress)
}

fn read_block_payload(
    drives: &mut [AtaDrive<SyscallBus>],
    device: u32,
    block: u64,
    count: u16,
) -> Result<Vec<u8>, DriveFault> {
    let lba = block_lba(block, count).map_err(|_| DriveFault::BadAddress)?;
    let engine = block_drive_of(drives, device).ok_or(DriveFault::NotFound)?;
    let mut buffer = vec![0u8; count as usize * SECTOR_BYTES];
    engine.read_sectors(lba, count as u8, &mut buffer)?;
    Ok(buffer)
}

fn write_block_payload(
    drives: &mut [AtaDrive<SyscallBus>],
    device: u32,
    block: u64,
    count: u16,
    token: u32,
) -> Result<(), block_wire::BlockError> {
    let lba = block_lba(block, count)?;
    let payload = drain_incoming(token).ok_or(block_wire::BlockError::Protocol)?;
    if payload.len() != count as usize * SECTOR_BYTES {
        return Err(block_wire::BlockError::Protocol);
    }
    let engine = block_drive_of(drives, device).ok_or(block_wire::BlockError::NotFound)?;
    engine
        .write_sectors(lba, &payload)
        .map_err(|fault| block_error(&fault))
}

fn attach_block_payload(
    held: &mut Vec<(u32, UserDataHandle)>,
    connection: UserConnectionHandle,
    owner: u32,
    payload: &[u8],
) -> Result<block_wire::Reply<'static>, block_wire::BlockError> {
    match Syscall::ipc_data_write(connection, payload) {
        Err(_) => Err(block_wire::BlockError::Busy),
        Ok(object) => {
            let token = data_token(&object);
            held.push((owner, object));
            Ok(block_wire::Reply::Data { token })
        }
    }
}

fn encode_block_reply(reply: &block_wire::Reply<'_>) -> [u8; block_wire::FRAME_BYTES] {
    reply
        .encode()
        .unwrap_or_else(|_| encode_block_protocol_error())
}

fn encode_block_protocol_error() -> [u8; block_wire::FRAME_BYTES] {
    block_wire::Reply::Err(block_wire::BlockError::Protocol)
        .encode()
        .expect("protocol error reply always encodes")
}

fn error_reply(error: AtaError, error_register: u8, status: u8) -> Reply<'static> {
    Reply::Err {
        error,
        error_register,
        status,
    }
}
