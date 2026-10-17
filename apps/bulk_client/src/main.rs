#![no_std]
#![no_main]

extern crate alloc;

use alloc::vec;
use alloc::vec::Vec;
use usrlib::error::DataError;
use usrlib::handle::{UserConnectionHandle, UserDataHandle};
use usrlib::ipc::{ipc_receive_value, send_data};
use usrlib::println;
use usrlib::syscall::Syscall;

const MAX_DATA_BYTES: usize = 64 * 1024;
const OBJECT_LENGTHS: [usize; 3] = [32768, 32768, 8192];
const TOTAL_BYTES: usize = 73728;
const STARTUP_DELAY_MS: u64 = 40_000;

fn payload_for(object_index: usize, length: usize) -> Vec<u8> {
    (0..length).map(|byte_index| (byte_index * 7 + object_index) as u8).collect()
}

fn connect_to_bulk_echo() -> UserConnectionHandle {
    loop {
        if let Ok(connection) = Syscall::ipc_connect("BULKECHO") {
            return connection;
        }
        Syscall::sleep(100);
    }
}

fn probe_oversized_write(connection: UserConnectionHandle) {
    let payload = vec![0u8; MAX_DATA_BYTES + 1];
    match Syscall::ipc_data_write(connection, &payload) {
        Err(DataError::TooLarge) => println!("BULK TooLarge rejected"),
        _ => println!("BULK Mismatch toolarge"),
    }
}

fn probe_budget(connection: UserConnectionHandle) {
    let chunk = vec![1u8; MAX_DATA_BYTES];
    let mut held: Vec<UserDataHandle> = Vec::new();
    loop {
        match Syscall::ipc_data_write(connection, &chunk) {
            Ok(handle) => held.push(handle),
            Err(DataError::BudgetExceeded) => {
                println!("BULK Budget rejected");
                drop(held);
                break;
            }
            Err(_) => {
                println!("BULK Mismatch budget");
                return;
            }
        }
    }
    match Syscall::ipc_data_write(connection, &chunk) {
        Ok(mut handle) => {
            println!("BULK Refund ok");
            let _ = handle.release();
        }
        Err(_) => println!("BULK Mismatch refund"),
    }
}

fn verification_round(connection: UserConnectionHandle) {
    let mut handles: Vec<UserDataHandle> = Vec::new();
    let mut echoed_total = 0usize;
    let mut verified = true;
    for (object_index, length) in OBJECT_LENGTHS.iter().enumerate() {
        let payload = payload_for(object_index, *length);
        let Ok(handle) = send_data(connection, &payload) else {
            verified = false;
            break;
        };
        handles.push(handle);
        match ipc_receive_value(connection) {
            Ok(echoed) if echoed == *length => echoed_total += echoed,
            _ => {
                verified = false;
                break;
            }
        }
    }
    if let Some(first) = handles.first_mut() {
        let _ = first.release();
    }
    drop(handles);
    if verified && echoed_total == TOTAL_BYTES {
        println!("BULK Verified: {TOTAL_BYTES}");
    } else {
        println!("BULK Mismatch verify");
    }
}

#[unsafe(no_mangle)]
fn main() {
    Syscall::sleep(STARTUP_DELAY_MS);
    let connection = connect_to_bulk_echo();
    probe_oversized_write(connection);
    probe_budget(connection);
    verification_round(connection);
    Syscall::ipc_disconnect(connection);
    println!("BULK Done");
    loop {
        Syscall::sleep(1000);
    }
}
