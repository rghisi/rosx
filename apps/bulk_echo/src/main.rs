#![no_std]
#![no_main]

extern crate alloc;

use core::alloc::{GlobalAlloc, Layout};
use core::panic::PanicInfo;
use usrlib::ipc::{ipc_send_value, receive_data};
use usrlib::println;
use usrlib::syscall::Syscall;

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

fn checksum(payload: &[u8]) -> u32 {
    payload
        .iter()
        .fold(0u32, |sum, byte| sum.wrapping_add(*byte as u32))
}

struct BulkEchoServer;

impl BulkEchoServer {
    fn run(&self) {
        if let Ok(binding) = Syscall::ipc_bind("BULKECHO") {
            loop {
                if let Some(fh) = Syscall::ipc_accept_message(binding) {
                    if let Ok(received) = Syscall::wait_for_message(fh) {
                        self.echo(&received);
                    }
                }
            }
        }
    }

    fn echo(&self, received: &usrlib::ipc::ReceivedMessage) {
        if let Ok(payload) = receive_data(received) {
            println!("BULK Echo: {} {}", payload.len(), checksum(&payload));
            if let Some(connection) = received.connection() {
                let _ = ipc_send_value(connection, payload.len());
            }
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn _start() {
    println!("[IPC Server] BulkEcho");
    let server = BulkEchoServer;
    server.run();
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    println!("[IPC Server] BulkEcho - Panic!");
    loop {

    }
}
