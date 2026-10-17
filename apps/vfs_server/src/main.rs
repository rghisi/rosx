#![no_std]
#![no_main]

extern crate alloc;

use alloc::format;
use alloc::vec::Vec;
use core::alloc::{GlobalAlloc, Layout};
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

const DRAIN_CHUNK_BYTES: usize = 4096;
const DRAIN_MAX_CHUNKS: usize = 16;

fn data_token(handle: &UserDataHandle) -> u32 {
    u32::from_str_radix(&format!("{}", handle), 16).expect("canonical data token")
}

fn conn_token(connection: &UserConnectionHandle) -> u32 {
    u32::from_str_radix(&format!("{}", connection), 16).expect("canonical connection token")
}

fn err_frame(error: VfsError) -> [u8; 60] {
    let mut frame = [0u8; 60];
    frame[0] = wire::STATUS_ERR;
    frame[1] = error as u8;
    frame
}

fn eof_frame() -> [u8; 60] {
    let mut frame = [0u8; 60];
    frame[0] = wire::STATUS_EOF;
    frame
}

fn opened_frame(handle: u32) -> [u8; 60] {
    let mut frame = [0u8; 60];
    frame[1..5].copy_from_slice(&handle.to_le_bytes());
    frame
}

fn written_frame(bytes: u32) -> [u8; 60] {
    let mut frame = [0u8; 60];
    frame[1..5].copy_from_slice(&bytes.to_le_bytes());
    frame
}

fn data_frame(token: u32, count: u32) -> [u8; 60] {
    let mut frame = [0u8; 60];
    frame[1..5].copy_from_slice(&token.to_le_bytes());
    frame[5..9].copy_from_slice(&count.to_le_bytes());
    frame
}

struct VfsServer {
    store: RamFs,
    held: Vec<(u32, UserDataHandle)>,
}

impl VfsServer {
    fn new() -> Self {
        Self {
            store: RamFs::new(),
            held: Vec::new(),
        }
    }

    fn run(&mut self) {
        if let Ok(binding) = Syscall::ipc_bind("VFS") {
            loop {
                if let Some(fh) = Syscall::ipc_accept_message(binding) {
                    if let Ok(received) = Syscall::wait_for_message(fh) {
                        self.handle_request(&received);
                    }
                }
            }
        }
    }

    fn handle_request(&mut self, received: &usrlib::ipc::ReceivedMessage) {
        let Some(connection) = received.connection() else {
            return;
        };
        let owner = conn_token(&connection);
        self.release_held_for(owner);
        let Some(request) = wire::decode_request(received.data()) else {
            return;
        };
        match request {
            Request::Open { mode, name } => {
                let frame = match self.store.open(owner, mode, name) {
                    Ok(handle) => opened_frame(handle),
                    Err(error) => err_frame(error),
                };
                self.send(connection, &frame);
            }
            Request::Read {
                handle,
                offset,
                count,
            } => self.read(connection, owner, handle, offset, count),
            Request::Write {
                handle,
                offset,
                token,
            } => self.write(connection, owner, handle, offset, token),
            Request::Close { handle } => {
                let frame = match self.store.close(owner, handle) {
                    Ok(()) => [0u8; 60],
                    Err(error) => err_frame(error),
                };
                self.send(connection, &frame);
            }
            Request::Delete { name } => {
                let frame = match self.store.delete(name) {
                    Ok(()) => [0u8; 60],
                    Err(error) => err_frame(error),
                };
                self.send(connection, &frame);
            }
            Request::List => self.list(connection, owner),
        }
    }

    fn read(
        &mut self,
        connection: UserConnectionHandle,
        owner: u32,
        handle: u32,
        offset: u64,
        count: u32,
    ) {
        if count == 0 {
            self.send(connection, &eof_frame());
            return;
        }
        let count = count.min(wire::RPC_MAX_DATA_BYTES as u32);
        let frame = match self.store.read(owner, handle, offset, count) {
            Ok(bytes) if bytes.is_empty() => eof_frame(),
            Ok(bytes) => self.reply_with_data(connection, owner, &bytes),
            Err(error) => err_frame(error),
        };
        self.send(connection, &frame);
    }

    fn write(
        &mut self,
        connection: UserConnectionHandle,
        owner: u32,
        handle: u32,
        offset: u64,
        token: u32,
    ) {
        let Some(payload) = Self::drain(token) else {
            self.send(connection, &err_frame(VfsError::ServerResource));
            return;
        };
        let frame = match self.store.write(owner, handle, offset, &payload) {
            Ok(bytes) => written_frame(bytes as u32),
            Err(error) => err_frame(error),
        };
        self.send(connection, &frame);
    }

    fn list(&mut self, connection: UserConnectionHandle, owner: u32) {
        let names = self.store.list();
        if names.is_empty() {
            self.send(connection, &eof_frame());
            return;
        }
        let frame = self.reply_with_data(connection, owner, &names);
        self.send(connection, &frame);
    }

    fn reply_with_data(
        &mut self,
        connection: UserConnectionHandle,
        owner: u32,
        payload: &[u8],
    ) -> [u8; 60] {
        match Syscall::ipc_data_write(connection, payload) {
            Ok(data_handle) => {
                let frame = data_frame(data_token(&data_handle), payload.len() as u32);
                self.held.push((owner, data_handle));
                frame
            }
            Err(_) => err_frame(VfsError::ServerResource),
        }
    }

    fn drain(token: u32) -> Option<Vec<u8>> {
        let mut payload = Vec::new();
        for _ in 0..DRAIN_MAX_CHUNKS {
            let mut chunk = [0u8; DRAIN_CHUNK_BYTES];
            match Syscall::ipc_data_read(token as usize, &mut chunk) {
                Ok(0) => return Some(payload),
                Ok(copied) => payload.extend_from_slice(&chunk[..copied]),
                Err(_) => return None,
            }
        }
        Some(payload)
    }

    fn release_held_for(&mut self, owner: u32) {
        let mut retained = Vec::new();
        for (held_owner, mut handle) in self.held.drain(..) {
            if held_owner == owner {
                let _ = handle.release();
            } else {
                retained.push((held_owner, handle));
            }
        }
        self.held = retained;
    }

    fn send(&self, connection: UserConnectionHandle, frame: &[u8; 60]) {
        let _ = Syscall::ipc_send(connection, frame);
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
