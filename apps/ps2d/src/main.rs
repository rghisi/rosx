#![no_std]

extern crate alloc;

use input_protocol::wire;
use system::syscall_numbers::SyscallNum;
use usrlib::handle::UserConnectionHandle;
use usrlib::println;
use usrlib::syscall::Syscall;

const CONNECT_ATTEMPTS: usize = 50;
const CONNECT_RETRY_DELAY_MS: u64 = 100;

fn connect_to_inputd() -> Option<UserConnectionHandle> {
    for _ in 0..CONNECT_ATTEMPTS {
        if let Ok(connection) = Syscall::ipc_connect(wire::SERVICE_NAME) {
            return Some(connection);
        }
        Syscall::sleep(CONNECT_RETRY_DELAY_MS);
    }
    None
}

fn main() {
    println!("[PS2D] Keystroke relay");
    loop {
        let Some(connection) = connect_to_inputd() else {
            continue;
        };
        relay_keystrokes(connection);
    }
}

fn relay_keystrokes(connection: UserConnectionHandle) {
    loop {
        let word = usrlib::arch::raw_syscall(SyscallNum::ReadChar as usize, 0, 0, 0);
        let Some(c) = char::from_u32(word as u32) else {
            continue;
        };
        if Syscall::ipc_send(connection, &wire::encode_push(c)).is_err() {
            return;
        }
    }
}
