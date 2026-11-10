#![no_std]

extern crate alloc;

use alloc::vec::Vec;
use system::kstats::{
    KSTATSD_SERVICE_NAME, PROCESS_STATE_BLOCKED, PROCESS_STATE_CREATED, PROCESS_STATE_READY,
    PROCESS_STATE_RUNNING, decode_process_entry, decode_process_table_count,
    encode_process_table_request,
};
use usrlib::handle::UserConnectionHandle;
use usrlib::ipc::receive_data;
use usrlib::println;
use usrlib::syscall::Syscall;

const CONNECT_ATTEMPTS: usize = 20;

fn connect_to_kstatsd() -> Option<UserConnectionHandle> {
    for _ in 0..CONNECT_ATTEMPTS {
        if let Ok(connection) = Syscall::ipc_connect(KSTATSD_SERVICE_NAME) {
            return Some(connection);
        }
        Syscall::sleep(100);
    }
    None
}

fn fetch_process_table(connection: UserConnectionHandle) -> Option<Vec<u8>> {
    Syscall::ipc_send(connection, &encode_process_table_request()).ok()?;
    let future = Syscall::ipc_receive_message(connection)?;
    let reply = Syscall::wait_for_message(future).ok()?;
    receive_data(&reply).ok()
}

fn state_name(state: u8) -> &'static str {
    match state {
        PROCESS_STATE_RUNNING => "running",
        PROCESS_STATE_READY => "ready",
        PROCESS_STATE_BLOCKED => "blocked",
        PROCESS_STATE_CREATED => "created",
        _ => "unknown",
    }
}

fn in_kib(bytes: u64) -> u64 {
    bytes / 1024
}

fn report(blob: &[u8]) {
    let Some(count) = decode_process_table_count(blob) else {
        println!("KSTATSD process table unreadable");
        return;
    };
    println!("{:>4}  {:<7} {:>7} {:>9}  NAME", "PID", "STATE", "THREADS", "MEM(KiB)");
    let mut processes = 0usize;
    let mut total_kib = 0u64;
    for index in 0..count {
        let Some(entry) = decode_process_entry(blob, index) else {
            continue;
        };
        processes += 1;
        total_kib += in_kib(entry.mem_bytes);
        let name = core::str::from_utf8(entry.name).unwrap_or("<invalid>");
        println!(
            "{:>4}  {:<7} {:>7} {:>9}  {}",
            entry.pid,
            state_name(entry.state),
            entry.thread_count,
            in_kib(entry.mem_bytes),
            name
        );
    }
    println!("{} processes, {} KiB accounted", processes, total_kib);
}

fn main() {
    let Some(connection) = connect_to_kstatsd() else {
        println!("KSTATSD unavailable");
        return;
    };
    match fetch_process_table(connection) {
        Some(blob) => report(&blob),
        None => println!("KSTATSD process table request failed"),
    }
    Syscall::ipc_disconnect(connection);
}
