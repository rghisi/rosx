#![no_std]

extern crate alloc;

use alloc::format;
use alloc::vec::Vec;
use ata_protocol::wire::{self, AtaError, Reply, Request};
use atad::taskfile::{AtaDrive, Bus, DriveFault, Identity};
use usrlib::error::PortError;
use usrlib::handle::{UserBindingHandle, UserConnectionHandle, UserDataHandle};
use usrlib::println;
use usrlib::syscall::Syscall;

const PRIMARY_CHANNEL: &str = "ata0";
const REBIND_DELAY_MS: u64 = 100;
const INCOMING_READ_CHUNK_BYTES: usize = 4096;
const DENIED_PARK_DELAY_MS: u64 = 1000;

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
        match drive.probe() {
            Ok(identity) => report_ready(slot, &identity),
            Err(DriveFault::Denied) if slot == 0 => park_without_port_access(),
            Err(_) => {}
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

fn park_without_port_access() -> ! {
    println!("[ATAD] no port access");
    loop {
        Syscall::sleep(DENIED_PARK_DELAY_MS);
    }
}

fn main() {
    println!("[ATAD] ATA disk driver");
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
    let reply = match Request::decode(received.data()) {
        Ok(request) => dispatch(drives, connection, owner, request),
        Err(error) => error_reply(error, 0, 0),
    };
    if let Ok(frame) = reply.encode() {
        let _ = Syscall::ipc_send(connection, &frame);
    }
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
    drives: &mut [AtaDrive<SyscallBus>],
    connection: UserConnectionHandle,
    owner: u32,
    request: Request,
) -> Reply {
    match request {
        Request::Identify { drive } => match drive_of(drives, drive).and_then(|drive| drive.identity()) {
            Some(identity) => Reply::Ident {
                sectors: identity.sectors,
                firmware_rev: &identity.firmware_rev,
                model: &identity.model,
            },
            None => error_reply(AtaError::NotFound, 0, 0),
        },
        Request::Read { .. } | Request::Write { .. } | Request::Flush { .. } => {
            error_reply(AtaError::Protocol, 0, 0)
        }
    }
}

fn drive_of(
    drives: &mut [AtaDrive<SyscallBus>],
    drive: u8,
) -> Option<&mut AtaDrive<SyscallBus>> {
    drives.get_mut(drive as usize)
}

fn error_reply(error: AtaError, error_register: u8, status: u8) -> Reply<'static> {
    Reply::Err {
        error,
        error_register,
        status,
    }
}
