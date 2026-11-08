#![no_std]

extern crate alloc;

use alloc::format;
use alloc::vec::Vec;
use core::cmp::min;
use usrlib::handle::{UserConnectionHandle, UserDataHandle};
use usrlib::println;
use usrlib::syscall::Syscall;
use vfs_protocol::wire::{self, Reply, Request, VfsError};
use vfsd::fs::ActiveStore;

const INCOMING_READ_CHUNK_BYTES: usize = 4096;

struct VfsServer {
    fs: ActiveStore,
    held: Vec<(u32, UserDataHandle)>,
}

impl VfsServer {
    fn new() -> VfsServer {
        VfsServer {
            fs: ActiveStore::new(),
            held: Vec::new(),
        }
    }

    fn mount_rom(&mut self) {
        let image = usrlib::rom::read_image();
        if image.is_empty() {
            return;
        }
        if let Err(err) = self.fs.attach_rom(&image) {
            panic!("VFS rom mount error: {err:?}");
        }
    }

    fn run(&mut self) {
        if let Ok(binding) = Syscall::ipc_bind(wire::SERVICE_NAME) {
            loop {
                if let Some(future) = Syscall::ipc_accept_message(binding) {
                    if let Ok(received) = Syscall::wait_for_message(future) {
                        self.serve(&received);
                    }
                }
            }
        }
    }

    fn serve(&mut self, received: &usrlib::ipc::ReceivedMessage) {
        let Some(connection) = received.connection() else {
            return;
        };
        let owner = Self::handle_token(connection);
        self.release_held_for(owner);
        let reply = match Request::decode(received.data()) {
            Ok(request) => self.dispatch(connection, owner, request),
            Err(error) => Reply::Err(error),
        };
        let _ = Syscall::ipc_send(connection, &reply.encode());
    }

    fn release_held_for(&mut self, owner: u32) {
        self.held.retain_mut(|(held_owner, object)| {
            if *held_owner == owner {
                let _ = object.release();
                false
            } else {
                true
            }
        });
    }

    fn dispatch(
        &mut self,
        connection: UserConnectionHandle,
        owner: u32,
        request: Request<'_>,
    ) -> Reply {
        match request {
            Request::Open { mode, name } => match self.fs.open(owner, mode, name) {
                Ok(handle) => Reply::Opened { handle },
                Err(error) => Reply::Err(error),
            },
            Request::Read {
                handle,
                offset,
                count,
            } => {
                let bounded = min(count as usize, wire::RPC_MAX_DATA_BYTES);
                let data = self.fs.read(owner, handle, offset, bounded);
                self.payload_reply(connection, owner, data, |token, total| {
                    Reply::ReadData { token, count: total }
                })
            }
            Request::Write {
                handle,
                offset,
                token,
            } => self.write_reply(owner, handle, offset, token),
            Request::Close { handle } => match self.fs.close(owner, handle) {
                Ok(()) => Reply::Ack,
                Err(error) => Reply::Err(error),
            },
            Request::Delete { name } => match self.fs.delete(name) {
                Ok(()) => Reply::Ack,
                Err(error) => Reply::Err(error),
            },
            Request::Stat { name } => match self.fs.stat(name) {
                Ok((kind, size)) => Reply::Stat { kind, size },
                Err(error) => Reply::Err(error),
            },
            Request::List { path } => {
                let blob = self.fs.list(path);
                self.payload_reply(connection, owner, blob, |token, total| {
                    Reply::Listed { token, total }
                })
            }
            Request::Mount { device, dir } => match self.fs.mount_device(device, dir) {
                Ok(()) => {
                    println!("[VFSD] mounted rxfs volume '{device}' at {dir}");
                    Reply::Ack
                }
                Err(error) => Reply::Err(error),
            },
            Request::Format { device } => match self.fs.format_device(device) {
                Ok(()) => {
                    println!("[VFSD] formatted rxfs volume on device '{device}'");
                    Reply::Ack
                }
                Err(error) => Reply::Err(error),
            },
            Request::MkDir { path } => match self.fs.mkdir(path) {
                Ok(()) => Reply::Ack,
                Err(error) => Reply::Err(error),
            },
            Request::RmDir { path } => match self.fs.rmdir(path) {
                Ok(()) => Reply::Ack,
                Err(error) => Reply::Err(error),
            },
        }
    }

    fn payload_reply(
        &mut self,
        connection: UserConnectionHandle,
        owner: u32,
        payload: Result<Vec<u8>, VfsError>,
        reply_with: fn(u32, u32) -> Reply,
    ) -> Reply {
        let payload = match payload {
            Err(error) => return Reply::Err(error),
            Ok(payload) => payload,
        };
        if payload.is_empty() {
            return Reply::Eof;
        }
        match Syscall::ipc_data_write(connection, &payload) {
            Err(_) => Reply::Err(VfsError::ServerResource),
            Ok(object) => {
                let token = Self::handle_token(&object);
                let reply = reply_with(token, payload.len() as u32);
                self.held.push((owner, object));
                reply
            }
        }
    }

    fn write_reply(
        &mut self,
        owner: u32,
        handle: u32,
        offset: u64,
        token: u32,
    ) -> Reply {
        let payload = match Self::drain_incoming(token) {
            None => return Reply::Err(VfsError::BadHandle),
            Some(payload) => payload,
        };
        match self.fs.write(owner, handle, offset, &payload) {
            Ok(written) => Reply::Written { bytes: written as u32 },
            Err(error) => Reply::Err(error),
        }
    }

    fn handle_token(value: impl core::fmt::Display) -> u32 {
        u32::from_str_radix(&format!("{value}"), 16).expect("canonical handle token")
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
}

fn main() {
    println!("[IPC Server] VFS");
    let mut server = VfsServer::new();
    server.mount_rom();
    server.run();
}
