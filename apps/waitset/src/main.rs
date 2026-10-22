#![no_std]

extern crate alloc;

use alloc::format;
use alloc::string::String;
use system::error::ErrorCode;
use system::ipc::MESSAGE_PAYLOAD_BYTES;
use system::syscall_numbers::SyscallNum;
use usrlib::error::WaitsetError;
use usrlib::handle::{UserBindingHandle, UserConnectionHandle, UserFutureHandle};
use usrlib::println;
use usrlib::syscall::Syscall;

const SERVICE_PREFIX: &str = "WAITSET";
const SERVICE_BIND_ATTEMPTS: usize = 8;
const ROUND_TAGS: [u32; 3] = [11, 22, 33];
const GUARD_TAG: u32 = 44;
const MISSING_CHILD_PROBE: usize = 0x7FFF_FFFF;

fn verdict_word(ok: bool) -> &'static str {
    if ok { "ok" } else { "fail" }
}

fn service_name(attempt: usize) -> String {
    if attempt == 0 {
        String::from(SERVICE_PREFIX)
    } else {
        format!("{}{}", SERVICE_PREFIX, attempt)
    }
}

fn bind_free_service() -> Option<(UserBindingHandle, String)> {
    let mut attempt = 0;
    while attempt < SERVICE_BIND_ATTEMPTS {
        let name = service_name(attempt);
        if let Ok(binding) = Syscall::ipc_bind(name.as_str()) {
            return Some((binding, name));
        }
        attempt += 1;
    }
    None
}

fn connect_to_service(name: &str) -> UserConnectionHandle {
    loop {
        if let Ok(connection) = Syscall::ipc_connect(name) {
            return connection;
        }
        Syscall::sleep(100);
    }
}

fn waitset_round(
    set: UserFutureHandle,
    binding: UserBindingHandle,
    connection: UserConnectionHandle,
    number: usize,
    tag: u32,
) -> bool {
    let Some(child) = Syscall::ipc_accept_message(binding) else {
        return false;
    };
    if Syscall::waitset_attach(set, child, tag).is_err() {
        return false;
    }
    let mut data = [0u8; MESSAGE_PAYLOAD_BYTES];
    data[0] = number as u8;
    if Syscall::ipc_send(connection, &data).is_err() {
        return false;
    }
    let Ok((event_child, event_tag)) = Syscall::waitset_wait(set) else {
        return false;
    };
    if event_child != child || event_tag != tag {
        return false;
    }
    let Ok(pong) = Syscall::wait_for_message(child) else {
        return false;
    };
    pong.data()[0] == number as u8
}

fn nesting_is_rejected(set: UserFutureHandle) -> bool {
    Syscall::waitset_attach(set, set, 0) == Err(WaitsetError::NestingForbidden)
}

fn absent_child_is_rejected(set: UserFutureHandle) -> bool {
    let Ok(set_word) = u32::from_str_radix(&format!("{}", set), 16) else {
        return false;
    };
    usrlib::arch::raw_syscall(
        SyscallNum::WaitsetAttach as usize,
        set_word as usize,
        MISSING_CHILD_PROBE,
        99,
    ) == ErrorCode::WaitsetChildInvalid.to_reg()
}

fn waitfuture_on_waitset_is_guarded(
    set: UserFutureHandle,
    binding: UserBindingHandle,
    connection: UserConnectionHandle,
) -> bool {
    let Some(child) = Syscall::ipc_accept_message(binding) else {
        return false;
    };
    if Syscall::waitset_attach(set, child, GUARD_TAG).is_err() {
        return false;
    }
    if Syscall::wait_for_message(set).is_ok() {
        return false;
    }
    let mut data = [0u8; MESSAGE_PAYLOAD_BYTES];
    data[0] = GUARD_TAG as u8;
    if Syscall::ipc_send(connection, &data).is_err() {
        return false;
    }
    let Ok((event_child, event_tag)) = Syscall::waitset_wait(set) else {
        return false;
    };
    if event_child != child || event_tag != GUARD_TAG {
        return false;
    }
    let Ok(pong) = Syscall::wait_for_message(child) else {
        return false;
    };
    pong.data()[0] == GUARD_TAG as u8
}

fn main() {
    let Some((binding, service)) = bind_free_service() else {
        println!("waitset complete: 3 rounds fail");
        return;
    };
    let connection = connect_to_service(service.as_str());
    let Ok(set) = Syscall::waitset_create() else {
        println!("waitset complete: 3 rounds fail");
        return;
    };
    let mut rounds_ok = true;
    for index in 0..ROUND_TAGS.len() {
        let number = index + 1;
        let ok = waitset_round(set, binding, connection, number, ROUND_TAGS[index]);
        println!(
            "waitset round: {} tag: {} {}",
            number,
            ROUND_TAGS[index],
            verdict_word(ok)
        );
        rounds_ok = rounds_ok && ok;
    }
    let nesting_ok = nesting_is_rejected(set);
    println!(
        "waitset negative: nesting rejected {}",
        verdict_word(nesting_ok)
    );
    let absent_ok = absent_child_is_rejected(set);
    println!(
        "waitset negative: child rejected {}",
        verdict_word(absent_ok)
    );
    let guard_ok = waitfuture_on_waitset_is_guarded(set, binding, connection);
    println!(
        "waitset negative: waitfuture guarded {}",
        verdict_word(guard_ok)
    );
    Syscall::waitset_drop(set);
    Syscall::waitset_drop(set);
    println!("waitset negative: drop idempotent ok");
    let complete = rounds_ok && nesting_ok && absent_ok && guard_ok;
    println!("waitset complete: 3 rounds {}", verdict_word(complete));
    Syscall::ipc_disconnect(connection);
}
