use crate::ipc::{IpcBindingError, IpcConnectionError, IpcReceiveError, IpcSendError};
use collections::generational_arena::Handle;

pub use collections::generational_arena::ERROR_BIT;

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[repr(u32)]
pub enum ErrorCode {
    IpcServerNotFound = 0x0101,
    IpcConnectionCannotBeEstablished = 0x0102,
    IpcAlreadyBound = 0x0103,
    IpcSendConnectionNotFound = 0x0104,
    IpcSendConnectionCongested = 0x0105,
    IpcReceiveConnectionNotFound = 0x0106,
    IpcReceiveNoMessagesAvailable = 0x0107,
    IpcMailboxNotAvailable = 0x0108,
    Unknown = 0x7FFF_FFFF,
}

impl ErrorCode {
    pub const fn to_reg(self) -> usize {
        ERROR_BIT | (self as u32 as usize)
    }

    pub fn from_code(raw: usize) -> ErrorCode {
        match raw & 0x7FFF_FFFF {
            0x0101 => ErrorCode::IpcServerNotFound,
            0x0102 => ErrorCode::IpcConnectionCannotBeEstablished,
            0x0103 => ErrorCode::IpcAlreadyBound,
            0x0104 => ErrorCode::IpcSendConnectionNotFound,
            0x0105 => ErrorCode::IpcSendConnectionCongested,
            0x0106 => ErrorCode::IpcReceiveConnectionNotFound,
            0x0107 => ErrorCode::IpcReceiveNoMessagesAvailable,
            0x0108 => ErrorCode::IpcMailboxNotAvailable,
            _ => ErrorCode::Unknown,
        }
    }
}

mod private {
    pub trait Sealed {}
}

pub trait ErrorCodeOf: private::Sealed {
    fn to_error_code(&self) -> ErrorCode;
}

impl private::Sealed for IpcConnectionError {}

impl ErrorCodeOf for IpcConnectionError {
    fn to_error_code(&self) -> ErrorCode {
        match self {
            IpcConnectionError::ServerNotFound => ErrorCode::IpcServerNotFound,
            IpcConnectionError::ConnectionCannotBeEstablished => {
                ErrorCode::IpcConnectionCannotBeEstablished
            }
        }
    }
}

impl private::Sealed for IpcBindingError {}

impl ErrorCodeOf for IpcBindingError {
    fn to_error_code(&self) -> ErrorCode {
        match self {
            IpcBindingError::AlreadyBound => ErrorCode::IpcAlreadyBound,
        }
    }
}

impl private::Sealed for IpcSendError {}

impl ErrorCodeOf for IpcSendError {
    fn to_error_code(&self) -> ErrorCode {
        match self {
            IpcSendError::ConnectionNotFound => ErrorCode::IpcSendConnectionNotFound,
            IpcSendError::ConnectionCongested => ErrorCode::IpcSendConnectionCongested,
        }
    }
}

impl private::Sealed for IpcReceiveError {}

impl ErrorCodeOf for IpcReceiveError {
    fn to_error_code(&self) -> ErrorCode {
        match self {
            IpcReceiveError::ConnectionNotFound => ErrorCode::IpcReceiveConnectionNotFound,
            IpcReceiveError::NoMessagesAvailable => ErrorCode::IpcReceiveNoMessagesAvailable,
            IpcReceiveError::MailboxNotAvailable => ErrorCode::IpcMailboxNotAvailable,
        }
    }
}

pub fn into_reg<E: ErrorCodeOf>(result: Result<Handle, E>) -> usize {
    match result {
        Ok(handle) => handle.pack(),
        Err(error) => error.to_error_code().to_reg(),
    }
}

pub fn from_reg(raw: usize) -> Result<Handle, ErrorCode> {
    if raw & ERROR_BIT != 0 {
        return Err(ErrorCode::from_code(raw));
    }
    if Handle::is_handle(raw) {
        return Ok(Handle::unpack(raw));
    }
    Err(ErrorCode::Unknown)
}

#[cfg(test)]
mod tests {
    use super::*;

    const EDGE_HANDLES: [(u16, u16); 4] = [(0, 0), (65535, 0), (0, 32767), (65535, 32767)];

    const ALL_ERROR_CODES: [ErrorCode; 9] = [
        ErrorCode::IpcServerNotFound,
        ErrorCode::IpcConnectionCannotBeEstablished,
        ErrorCode::IpcAlreadyBound,
        ErrorCode::IpcSendConnectionNotFound,
        ErrorCode::IpcSendConnectionCongested,
        ErrorCode::IpcReceiveConnectionNotFound,
        ErrorCode::IpcReceiveNoMessagesAvailable,
        ErrorCode::IpcMailboxNotAvailable,
        ErrorCode::Unknown,
    ];

    #[test]
    fn from_reg_of_into_reg_roundtrips_ok_handles() {
        for (index, generation) in EDGE_HANDLES {
            let handle = Handle::new(index, generation);
            let raw = into_reg::<IpcConnectionError>(Ok(handle));
            assert_eq!(from_reg(raw), Ok(handle));
        }
    }

    #[test]
    fn every_error_code_variant_roundtrips_through_the_register_codec() {
        for code in ALL_ERROR_CODES {
            let raw = code.to_reg();
            assert_ne!(raw & ERROR_BIT, 0);
            assert_eq!(from_reg(raw), Err(code));
            assert_eq!(ErrorCode::from_code(raw), code);
        }
    }

    #[test]
    fn connection_error_variants_map_to_reserved_codes_and_decode_as_errors() {
        for error in [
            IpcConnectionError::ServerNotFound,
            IpcConnectionError::ConnectionCannotBeEstablished,
        ] {
            let raw = into_reg(Err(error));
            assert!(from_reg(raw).is_err());
            assert_eq!(
                error.to_error_code(),
                match error {
                    IpcConnectionError::ServerNotFound => ErrorCode::IpcServerNotFound,
                    IpcConnectionError::ConnectionCannotBeEstablished => {
                        ErrorCode::IpcConnectionCannotBeEstablished
                    }
                }
            );
            assert_eq!(ErrorCode::from_code(raw), error.to_error_code());
        }
    }

    #[test]
    fn binding_error_variants_map_to_reserved_codes_and_decode_as_errors() {
        for error in [IpcBindingError::AlreadyBound] {
            let raw = into_reg(Err(error));
            assert!(from_reg(raw).is_err());
            assert_eq!(error.to_error_code(), ErrorCode::IpcAlreadyBound);
            assert_eq!(ErrorCode::from_code(raw), error.to_error_code());
        }
    }

    #[test]
    fn send_error_variants_map_to_reserved_codes_and_decode_as_errors() {
        for error in [
            IpcSendError::ConnectionNotFound,
            IpcSendError::ConnectionCongested,
        ] {
            let raw = into_reg(Err(error.clone()));
            assert!(from_reg(raw).is_err());
            assert_eq!(
                error.to_error_code(),
                match error {
                    IpcSendError::ConnectionNotFound => ErrorCode::IpcSendConnectionNotFound,
                    IpcSendError::ConnectionCongested => ErrorCode::IpcSendConnectionCongested,
                }
            );
            assert_eq!(ErrorCode::from_code(raw), error.to_error_code());
        }
    }

    #[test]
    fn receive_error_variants_map_to_reserved_codes_and_decode_as_errors() {
        for error in [
            IpcReceiveError::ConnectionNotFound,
            IpcReceiveError::NoMessagesAvailable,
            IpcReceiveError::MailboxNotAvailable,
        ] {
            let raw = into_reg(Err(error));
            assert!(from_reg(raw).is_err());
            assert_eq!(
                error.to_error_code(),
                match error {
                    IpcReceiveError::ConnectionNotFound => ErrorCode::IpcReceiveConnectionNotFound,
                    IpcReceiveError::NoMessagesAvailable =>
                        ErrorCode::IpcReceiveNoMessagesAvailable,
                    IpcReceiveError::MailboxNotAvailable => ErrorCode::IpcMailboxNotAvailable,
                }
            );
            assert_eq!(ErrorCode::from_code(raw), error.to_error_code());
        }
    }

    #[test]
    fn from_code_is_total_and_decodes_unmatched_payloads_to_unknown() {
        assert_eq!(ErrorCode::from_code(0x1234), ErrorCode::Unknown);
        assert_eq!(ErrorCode::from_code(0x0001_0000), ErrorCode::Unknown);
        assert_eq!(ErrorCode::from_code(usize::MAX), ErrorCode::Unknown);
        assert_eq!(ErrorCode::from_code(ERROR_BIT | 0x1234), ErrorCode::Unknown);
    }

    #[test]
    fn from_reg_rejects_high_bit_garbage_and_exec_sentinel_as_unknown() {
        assert_eq!(from_reg(1usize << 40), Err(ErrorCode::Unknown));
        assert_eq!(from_reg(u64::MAX as usize), Err(ErrorCode::Unknown));
    }
}
