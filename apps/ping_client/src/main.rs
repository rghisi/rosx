#![no_std]

extern crate alloc;

use alloc::format;
use alloc::vec::Vec;
use system::ipc::MESSAGE_PAYLOAD_BYTES;
use usrlib::handle::{UserConnectionHandle, UserDataHandle};
use usrlib::ipc::receive_data;
use usrlib::println;
use usrlib::syscall::Syscall;

const MAX_DATA_BYTES: usize = 64 * 1024;
const PING_SIZES: [usize; 3] = [64, 4096, MAX_DATA_BYTES];

fn checksum(payload: &[u8]) -> u32 {
    payload
        .iter()
        .fold(0u32, |sum, byte| sum.wrapping_add(*byte as u32))
}

fn ping_byte(index: usize) -> u8 {
    (index as u32).wrapping_mul(31).wrapping_add(17) as u8
}

fn data_token(handle: &UserDataHandle) -> u32 {
    u32::from_str_radix(&format!("{}", handle), 16).expect("canonical data token")
}

fn verdict_word(ok: bool) -> &'static str {
    if ok { "ok" } else { "fail" }
}

fn connect_to_ping_server() -> UserConnectionHandle {
    loop {
        if let Ok(connection) = Syscall::ipc_connect("PING") {
            return connection;
        }
        Syscall::sleep(100);
    }
}

fn parse_pong(data: &[u8; MESSAGE_PAYLOAD_BYTES]) -> Option<(&str, usize, u32)> {
    let text = core::str::from_utf8(&data[4..])
        .ok()?
        .trim_end_matches('\0');
    let mut fields = text.split(':');
    if fields.next()? != "PONG" {
        return None;
    }
    let matched = fields.next()?;
    let size = fields.next()?.parse::<usize>().ok()?;
    let announced_checksum = u32::from_str_radix(fields.next()?, 16).ok()?;
    Some((matched, size, announced_checksum))
}

fn ping_round(connection: UserConnectionHandle, size: usize) {
    let payload: Vec<u8> = (0..size).map(ping_byte).collect();
    let Ok(mut ping_handle) = Syscall::ipc_data_write(connection, &payload) else {
        println!("ping sent: {size} fail received: 0 fail");
        return;
    };
    let mut data = [0u8; MESSAGE_PAYLOAD_BYTES];
    data[..4].copy_from_slice(&data_token(&ping_handle).to_ne_bytes());
    let text = format!("PING:{size}:{:08x}", checksum(&payload));
    data[4..4 + text.len()].copy_from_slice(text.as_bytes());
    if Syscall::ipc_send(connection, &data).is_err() {
        println!("ping sent: {size} fail received: 0 fail");
        return;
    }
    let Some(fh) = Syscall::ipc_receive_message(connection) else {
        println!("ping sent: {size} fail received: 0 fail");
        return;
    };
    let Ok(pong) = Syscall::wait_for_message(fh) else {
        println!("ping sent: {size} fail received: 0 fail");
        return;
    };
    let pong_bytes = match receive_data(&pong) {
        Ok(bytes) => bytes,
        Err(_) => {
            println!("ping sent: {size} fail received: 0 fail");
            return;
        }
    };
    let parsed = parse_pong(pong.data());
    let sent_ok = matches!(parsed, Some(("ok", _, _)));
    let received_ok =
        parsed.is_some_and(|(_word, _size, announced)| checksum(&pong_bytes) == announced);
    println!(
        "ping sent: {} {} received: {} {}",
        size,
        verdict_word(sent_ok),
        pong_bytes.len(),
        verdict_word(received_ok)
    );
    let _ = ping_handle.release();
}

fn main() {
    let connection = connect_to_ping_server();
    for size in PING_SIZES {
        ping_round(connection, size);
    }
    Syscall::ipc_disconnect(connection);
}
