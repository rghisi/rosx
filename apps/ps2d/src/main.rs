#![no_std]

extern crate alloc;

use input_protocol::wire;
use ps2d::decoder::Ps2Decoder;
use usrlib::handle::{UserBindingHandle, UserConnectionHandle};
use usrlib::println;
use usrlib::syscall::Syscall;

const CONNECT_ATTEMPTS: usize = 50;
const CONNECT_RETRY_DELAY_MS: u64 = 100;

fn main() {
    println!("[PS2D] PS/2 keyboard driver");
    let mut decoder = Ps2Decoder::new();
    loop {
        match Syscall::ipc_bind(wire::SOURCE_NAME_PS2KEYBOARD) {
            Ok(binding) => serve(binding, &mut decoder),
            Err(_) => Syscall::sleep(CONNECT_RETRY_DELAY_MS),
        }
    }
}

fn serve(binding: UserBindingHandle, decoder: &mut Ps2Decoder) {
    let mut inputd: Option<UserConnectionHandle> = None;
    loop {
        if let Some(future) = Syscall::ipc_accept_message(binding) {
            let Ok(received) = Syscall::wait_for_message(future) else {
                continue;
            };
            if received.connection().is_some() {
                continue;
            }
            let Some(c) = decoder.feed(wire::scancode_of(received.data())) else {
                continue;
            };
            push_char(&mut inputd, c);
        }
    }
}

fn push_char(inputd: &mut Option<UserConnectionHandle>, c: char) {
    if inputd.is_none() {
        *inputd = connect_to_inputd();
    }
    let Some(connection) = *inputd else {
        return;
    };
    if Syscall::ipc_send(connection, &wire::encode_push(c)).is_err() {
        *inputd = None;
    }
}

fn connect_to_inputd() -> Option<UserConnectionHandle> {
    for _ in 0..CONNECT_ATTEMPTS {
        if let Ok(connection) = Syscall::ipc_connect(wire::SERVICE_NAME) {
            return Some(connection);
        }
        Syscall::sleep(CONNECT_RETRY_DELAY_MS);
    }
    None
}
