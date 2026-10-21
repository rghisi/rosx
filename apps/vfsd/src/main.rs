#![no_std]

extern crate alloc;

use alloc::format;
use alloc::vec::Vec;
use core::cmp::min;
use usrlib::handle::{UserConnectionHandle, UserDataHandle};
use usrlib::println;
use usrlib::syscall::Syscall;
use vfs_protocol::wire::{self, Reply, Request, VfsError};
use vfsd::fs::RamFs;

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

fn read_rom_image() -> Vec<u8> {
    let mut image = Vec::new();
    let mut offset: u64 = 0;
    loop {
        let mut chunk = [0u8; INCOMING_READ_CHUNK_BYTES];
        let copied = Syscall::rom_read(offset, &mut chunk);
        if copied == 0 {
            return image;
        }
        image.extend_from_slice(&chunk[..copied]);
        offset += copied as u64;
    }
}

struct VfsServer {
    fs: RamFs,
    held: Vec<(u32, UserDataHandle)>,
}

impl VfsServer {
    fn new() -> VfsServer {
        VfsServer {
            fs: RamFs::new(),
            held: Vec::new(),
        }
    }

    fn mount_rom(&mut self) {
        let image = read_rom_image();
        if image.is_empty() {
            return;
        }
        if let Err(err) = self.fs.attach_rom(image) {
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
        let owner = conn_token(connection);
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
            } => self.read_reply(connection, owner, handle, offset, count),
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
            Request::List => self.list_reply(connection, owner),
        }
    }

    fn read_reply(
        &mut self,
        connection: UserConnectionHandle,
        owner: u32,
        handle: u32,
        offset: u64,
        count: u32,
    ) -> Reply {
        if count == 0 {
            return Reply::Eof;
        }
        let bounded = min(count as usize, wire::RPC_MAX_DATA_BYTES);
        let data = match self.fs.read(owner, handle, offset, bounded) {
            Err(error) => return Reply::Err(error),
            Ok(data) => data,
        };
        if data.is_empty() {
            return Reply::Eof;
        }
        self.attach_payload(connection, owner, &data, |token, total| {
            Reply::ReadData { token, count: total }
        })
    }

    fn list_reply(
        &mut self,
        connection: UserConnectionHandle,
        owner: u32,
    ) -> Reply {
        let blob = match self.fs.list() {
            Err(error) => return Reply::Err(error),
            Ok(blob) => blob,
        };
        if blob.is_empty() {
            return Reply::Eof;
        }
        self.attach_payload(connection, owner, &blob, |token, total| {
            Reply::Listed { token, total }
        })
    }

    fn attach_payload(
        &mut self,
        connection: UserConnectionHandle,
        owner: u32,
        payload: &[u8],
        reply_with: fn(u32, u32) -> Reply,
    ) -> Reply {
        match Syscall::ipc_data_write(connection, payload) {
            Err(_) => Reply::Err(VfsError::ServerResource),
            Ok(object) => {
                let token = data_token(&object);
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
        let payload = match drain_incoming(token) {
            None => return Reply::Err(VfsError::BadHandle),
            Some(payload) => payload,
        };
        match self.fs.write(owner, handle, offset, &payload) {
            Ok(written) => Reply::Written { bytes: written as u32 },
            Err(error) => Reply::Err(error),
        }
    }
}

fn main() {
    println!("[IPC Server] VFS");
    let mut server = VfsServer::new();
    server.mount_rom();
    server.run();
}
