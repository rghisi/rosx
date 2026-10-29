use crate::handle::UserConnectionHandle;
use crate::ipc::ReceivedMessage;
use crate::syscall::Syscall;
use core::sync::atomic::{AtomicUsize, Ordering};
use input_protocol::wire;
use system::ipc::MESSAGE_PAYLOAD_BYTES;

const CONNECT_ATTEMPTS: usize = 50;
const CONNECT_RETRY_DELAY_MS: u64 = 100;
const NO_CACHED_CONNECTION: usize = usize::MAX;

static CACHED_CONNECTION: AtomicUsize = AtomicUsize::new(NO_CACHED_CONNECTION);

pub(crate) fn read_char() -> char {
    let Some(received) = transact(&wire::encode_read()) else {
        return '\0';
    };
    if wire::frame_tag(received.data()) != wire::TAG_KEY {
        return '\0';
    }
    wire::frame_char(received.data()).unwrap_or('\0')
}

pub(crate) fn try_read_char() -> Option<char> {
    let received = transact(&wire::encode_read_nowait())?;
    if wire::frame_tag(received.data()) != wire::TAG_KEY {
        return None;
    }
    wire::frame_char(received.data())
}

fn transact(request: &[u8; MESSAGE_PAYLOAD_BYTES]) -> Option<ReceivedMessage> {
    let connection = ensure_connection()?;
    if Syscall::ipc_send(connection, request).is_err() {
        invalidate_connection();
        return None;
    }
    let Some(future) = Syscall::ipc_receive_message(connection) else {
        invalidate_connection();
        return None;
    };
    match Syscall::wait_for_message(future) {
        Ok(received) => Some(received),
        Err(_) => {
            invalidate_connection();
            None
        }
    }
}

fn ensure_connection() -> Option<UserConnectionHandle> {
    let cached = CACHED_CONNECTION.load(Ordering::Relaxed);
    if let Some(connection) = UserConnectionHandle::from_token(cached) {
        return Some(connection);
    }
    for _ in 0..CONNECT_ATTEMPTS {
        if let Ok(connection) = Syscall::ipc_connect(wire::SERVICE_NAME) {
            CACHED_CONNECTION.store(connection.packed() as usize, Ordering::Relaxed);
            return Some(connection);
        }
        Syscall::sleep(CONNECT_RETRY_DELAY_MS);
    }
    None
}

fn invalidate_connection() {
    CACHED_CONNECTION.store(NO_CACHED_CONNECTION, Ordering::Relaxed);
}
