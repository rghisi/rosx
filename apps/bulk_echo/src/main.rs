#![no_std]
#![no_main]

extern crate alloc;

use usrlib::ipc::{ipc_send_value, receive_data};
use usrlib::println;
use usrlib::syscall::Syscall;

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

usrlib::app_main! { fn main(_args: Vec<String>) {
    println!("[IPC Server] BulkEcho");
    let server = BulkEchoServer;
    server.run();
} }
