#![no_std]

extern crate alloc;

use input_protocol::wire;
use inputd::mux::{InputMux, MuxAction};
use usrlib::handle::{UserBindingHandle, UserConnectionHandle};
use usrlib::ipc::ReceivedMessage;
use usrlib::println;
use usrlib::syscall::Syscall;

const BIND_RETRY_DELAY_MS: u64 = 100;

fn main() {
    println!("[INPUTD] Input multiplexer");
    loop {
        match Syscall::ipc_bind(wire::SERVICE_NAME) {
            Ok(binding) => serve(binding),
            Err(_) => Syscall::sleep(BIND_RETRY_DELAY_MS),
        }
    }
}

fn serve(binding: UserBindingHandle) {
    let mut mux: InputMux<UserConnectionHandle> = InputMux::new();
    loop {
        if let Some(future) = Syscall::ipc_accept_message(binding) {
            if let Ok(received) = Syscall::wait_for_message(future) {
                serve_one(&mut mux, &received);
            }
        }
    }
}

fn serve_one(mux: &mut InputMux<UserConnectionHandle>, received: &ReceivedMessage) {
    let Some(connection) = received.connection() else {
        return;
    };
    let action = match wire::frame_tag(received.data()) {
        wire::TAG_READ => mux.handle_read(connection),
        wire::TAG_READ_NOWAIT => mux.handle_read_nowait(connection),
        wire::TAG_PUSH => match wire::frame_char(received.data()) {
            Some(c) => mux.handle_push(c),
            None => None,
        },
        _ => None,
    };
    if let Some(MuxAction::Reply(reader, frame)) = action {
        if Syscall::ipc_send(reader, &frame).is_err() {
            mux.forget(reader);
        }
    }
}
