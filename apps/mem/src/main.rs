#![no_std]

extern crate alloc;

use alloc::format;
use system::kstats::{
    KSTATSD_SERVICE_NAME, MemorySnapshot, decode_memory_snapshot, encode_memory_request,
};
use usrlib::handle::UserConnectionHandle;
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

fn fetch_snapshot(connection: UserConnectionHandle) -> Option<MemorySnapshot> {
    Syscall::ipc_send(connection, &encode_memory_request()).ok()?;
    let future = Syscall::ipc_receive_message(connection)?;
    let reply = Syscall::wait_for_message(future).ok()?;
    decode_memory_snapshot(reply.data())
}

fn in_kib(bytes: u64) -> u64 {
    bytes / 1024
}

fn used_percentage(snapshot: &MemorySnapshot) -> u64 {
    if snapshot.total_bytes == 0 {
        0
    } else {
        snapshot.used_bytes * 100 / snapshot.total_bytes
    }
}

fn report(snapshot: &MemorySnapshot) {
    println!("KSTATSD memory snapshot");
    println!(
        "  used:     {} KiB ({}%)",
        in_kib(snapshot.used_bytes),
        used_percentage(snapshot)
    );
    println!("  free:     {} KiB", in_kib(snapshot.free_bytes));
    println!("  total:    {} KiB", in_kib(snapshot.total_bytes));
    println!(
        "  largest free block: {} KiB",
        in_kib(snapshot.largest_free_block)
    );
    println!("  free blocks: {}", snapshot.free_block_count);
    println!("  uptime: {} s", snapshot.uptime_ms / 1000);
}

fn main() {
    let Some(connection) = connect_to_kstatsd() else {
        println!("KSTATSD unavailable");
        return;
    };
    match fetch_snapshot(connection) {
        Some(snapshot) => report(&snapshot),
        None => println!("KSTATSD request failed"),
    }
    Syscall::ipc_disconnect(connection);
}
