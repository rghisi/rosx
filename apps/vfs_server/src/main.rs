#![no_std]
#![no_main]

extern crate alloc;

use alloc::format;
use alloc::vec::Vec;
use core::alloc::{GlobalAlloc, Layout};
use core::cmp::min;
use core::panic::PanicInfo;
use usrlib::handle::{UserConnectionHandle, UserDataHandle};
use usrlib::println;
use usrlib::syscall::Syscall;
use vfs_protocol::wire::{self, Request, VfsError};
use vfs_server::fs::RamFs;

struct SyscallAllocator;

unsafe impl GlobalAlloc for SyscallAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        Syscall::alloc(layout.size(), layout.align())
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        Syscall::dealloc(ptr, layout.size(), layout.align());
    }
}

#[global_allocator]
static ALLOCATOR: SyscallAllocator = SyscallAllocator;

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
        let reply = match wire::decode_request(received.data()) {
            Ok(request) => self.dispatch(connection, owner, request),
            Err(error) => wire::encode_err(error),
        };
        let _ = Syscall::ipc_send(connection, &reply);
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
    ) -> [u8; wire::FRAME_BYTES] {
        match request {
            Request::Open { mode, name } => match self.fs.open(owner, mode, name) {
                Ok(handle) => wire::encode_opened(handle),
                Err(error) => wire::encode_err(error),
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
                Ok(()) => wire::encode_ack(),
                Err(error) => wire::encode_err(error),
            },
            Request::Delete { name } => match self.fs.delete(name) {
                Ok(()) => wire::encode_ack(),
                Err(error) => wire::encode_err(error),
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
    ) -> [u8; wire::FRAME_BYTES] {
        if count == 0 {
            return wire::encode_eof();
        }
        let bounded = min(count as usize, wire::RPC_MAX_DATA_BYTES);
        let data = match self.fs.read(owner, handle, offset, bounded) {
            Err(error) => return wire::encode_err(error),
            Ok(data) => data,
        };
        if data.is_empty() {
            return wire::encode_eof();
        }
        self.attach_payload(connection, owner, &data, wire::encode_read_data)
    }

    fn list_reply(
        &mut self,
        connection: UserConnectionHandle,
        owner: u32,
    ) -> [u8; wire::FRAME_BYTES] {
        let blob = match self.fs.list() {
            Err(error) => return wire::encode_err(error),
            Ok(blob) => blob,
        };
        if blob.is_empty() {
            return wire::encode_eof();
        }
        self.attach_payload(connection, owner, &blob, wire::encode_listed)
    }

    fn attach_payload(
        &mut self,
        connection: UserConnectionHandle,
        owner: u32,
        payload: &[u8],
        frame_with: fn(u32, u32) -> [u8; wire::FRAME_BYTES],
    ) -> [u8; wire::FRAME_BYTES] {
        match Syscall::ipc_data_write(connection, payload) {
            Err(_) => wire::encode_err(VfsError::ServerResource),
            Ok(object) => {
                let token = data_token(&object);
                let frame = frame_with(token, payload.len() as u32);
                self.held.push((owner, object));
                frame
            }
        }
    }

    fn write_reply(
        &mut self,
        owner: u32,
        handle: u32,
        offset: u64,
        token: u32,
    ) -> [u8; wire::FRAME_BYTES] {
        let payload = match drain_incoming(token) {
            None => return wire::encode_err(VfsError::BadHandle),
            Some(payload) => payload,
        };
        match self.fs.write(owner, handle, offset, &payload) {
            Ok(written) => wire::encode_written(written as u32),
            Err(error) => wire::encode_err(error),
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn _start() {
    println!("[IPC Server] VFS");
    let mut server = VfsServer::new();
    server.run();
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    println!("[IPC Server] VFS - Panic!");
    loop {}
}
