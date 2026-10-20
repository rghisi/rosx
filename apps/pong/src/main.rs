#![no_std]

extern crate alloc;

use alloc::format;
use alloc::vec::Vec;
use system::ipc::MESSAGE_PAYLOAD_BYTES;
use usrlib::handle::UserDataHandle;
use usrlib::ipc::receive_data;
use usrlib::println;
use usrlib::syscall::Syscall;

fn checksum(payload: &[u8]) -> u32 {
    payload
        .iter()
        .fold(0u32, |sum, byte| sum.wrapping_add(*byte as u32))
}

fn pong_byte(index: usize) -> u8 {
    (index as u32).wrapping_mul(37).wrapping_add(23) as u8
}

fn data_token(handle: &UserDataHandle) -> u32 {
    u32::from_str_radix(&format!("{}", handle), 16).expect("canonical data token")
}

fn announced_checksum(data: &[u8; MESSAGE_PAYLOAD_BYTES]) -> Option<u32> {
    let text = core::str::from_utf8(&data[4..])
        .ok()?
        .trim_end_matches('\0');
    let mut fields = text.split(':');
    fields.next()?;
    fields.next()?;
    u32::from_str_radix(fields.next()?, 16).ok()
}

struct PingServer {
    previous_pong: Option<UserDataHandle>,
}

impl PingServer {
    fn new() -> Self {
        Self {
            previous_pong: None,
        }
    }

    fn run(&mut self) {
        if let Ok(binding) = Syscall::ipc_bind("PING") {
            loop {
                if let Some(fh) = Syscall::ipc_accept_message(binding) {
                    if let Ok(received) = Syscall::wait_for_message(fh) {
                        self.reply(&received);
                    }
                }
            }
        }
    }

    fn reply(&mut self, received: &usrlib::ipc::ReceivedMessage) {
        if let Some(mut held) = self.previous_pong.take() {
            let _ = held.release();
        }
        let Ok(ping_payload) = receive_data(received) else {
            return;
        };
        let matched = announced_checksum(received.data()) == Some(checksum(&ping_payload));
        let pong_payload: Vec<u8> = (0..ping_payload.len()).map(pong_byte).collect();
        let verdict_word = if matched { "ok" } else { "fail" };
        if let Some(connection) = received.connection() {
            if let Ok(pong_handle) = Syscall::ipc_data_write(connection, &pong_payload) {
                let mut data = [0u8; MESSAGE_PAYLOAD_BYTES];
                data[..4].copy_from_slice(&data_token(&pong_handle).to_ne_bytes());
                let text = format!(
                    "PONG:{}:{}:{:08x}",
                    verdict_word,
                    pong_payload.len(),
                    checksum(&pong_payload)
                );
                data[4..4 + text.len()].copy_from_slice(text.as_bytes());
                let _ = Syscall::ipc_send(connection, &data);
                println!("PONG {} matched {}", pong_payload.len(), verdict_word);
                self.previous_pong = Some(pong_handle);
            }
        }
    }
}

fn main() {
    println!("[IPC Server] Ping");
    let mut server = PingServer::new();
    server.run();
}
