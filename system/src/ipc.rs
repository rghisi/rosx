pub const MESSAGE_BYTES: usize = 64;
pub const MESSAGE_PAYLOAD_BYTES: usize = 60;

#[repr(C)]
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct Message {
    conn: u32,
    data: [u8; MESSAGE_PAYLOAD_BYTES],
}

impl Message {
    pub const EMPTY: Self = Self { conn: 0, data: [0; MESSAGE_PAYLOAD_BYTES] };

    pub fn new(conn: u32, data: &[u8; MESSAGE_PAYLOAD_BYTES]) -> Self {
        Self { conn, data: *data }
    }

    pub const fn conn(&self) -> u32 {
        self.conn
    }

    pub fn data(&self) -> &[u8; MESSAGE_PAYLOAD_BYTES] {
        &self.data
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::{ERROR_BIT, ErrorCode, is_token};

    #[test]
    fn message_is_64_bytes_align_4() {
        assert_eq!(core::mem::size_of::<Message>(), 64);
        assert_eq!(core::mem::align_of::<Message>(), 4);
    }

    #[test]
    fn message_conn_field_at_offset_zero() {
        assert_eq!(core::mem::offset_of!(Message, conn), 0);
        assert_eq!(core::mem::offset_of!(Message, data), 4);
    }

    #[test]
    fn message_new_and_accessors_roundtrip() {
        let pattern: [u8; MESSAGE_PAYLOAD_BYTES] = core::array::from_fn(|i| i as u8);
        let message = Message::new(0x7FFF_FFFF, &pattern);
        assert_eq!(message.conn(), 0x7FFF_FFFF);
        assert_eq!(*message.data(), pattern);
    }

    #[test]
    fn packed_handle_space_is_disjoint_from_error_space() {
        assert!(is_token(0x7FFF_FFFF));
        assert!(!is_token(ERROR_BIT));
        for code in [
            ErrorCode::IpcServerNotFound,
            ErrorCode::IpcConnectionCannotBeEstablished,
            ErrorCode::IpcAlreadyBound,
            ErrorCode::IpcSendConnectionNotFound,
            ErrorCode::IpcSendConnectionCongested,
            ErrorCode::IpcReceiveConnectionNotFound,
            ErrorCode::IpcReceiveNoMessagesAvailable,
            ErrorCode::IpcMailboxNotAvailable,
            ErrorCode::IpcDataTooLarge,
            ErrorCode::IpcDataBudgetExceeded,
            ErrorCode::IpcDataNotFound,
            ErrorCode::IpcDataNotReadable,
            ErrorCode::Unknown,
        ] {
            let raw = code.to_reg();
            assert_ne!(raw & ERROR_BIT, 0);
            assert!(!is_token(raw));
            assert_eq!(ErrorCode::from_code(raw), code);
        }
    }
}
