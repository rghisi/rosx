pub const ERROR_BIT: usize = 1 << 31;

pub const fn is_token(raw: usize) -> bool {
    raw < ERROR_BIT
}

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
    IpcDataTooLarge = 0x0109,
    IpcDataBudgetExceeded = 0x010A,
    IpcDataNotFound = 0x010B,
    IpcDataNotReadable = 0x010C,
    ElfArgumentLimitExceeded = 0x0201,
    ElfArgumentBlobMalformed = 0x0202,
    PortAccessDenied = 0x0301,
    PortRequestInvalid = 0x0302,
    Unknown = 0x7FFF_FFFF,
}

impl ErrorCode {
    pub const fn to_reg(self) -> usize {
        ERROR_BIT | (self as u32 as usize)
    }

    pub fn from_code(raw: usize) -> ErrorCode {
        match raw & (ERROR_BIT - 1) {
            0x0101 => ErrorCode::IpcServerNotFound,
            0x0102 => ErrorCode::IpcConnectionCannotBeEstablished,
            0x0103 => ErrorCode::IpcAlreadyBound,
            0x0104 => ErrorCode::IpcSendConnectionNotFound,
            0x0105 => ErrorCode::IpcSendConnectionCongested,
            0x0106 => ErrorCode::IpcReceiveConnectionNotFound,
            0x0107 => ErrorCode::IpcReceiveNoMessagesAvailable,
            0x0108 => ErrorCode::IpcMailboxNotAvailable,
            0x0109 => ErrorCode::IpcDataTooLarge,
            0x010A => ErrorCode::IpcDataBudgetExceeded,
            0x010B => ErrorCode::IpcDataNotFound,
            0x010C => ErrorCode::IpcDataNotReadable,
            0x0201 => ErrorCode::ElfArgumentLimitExceeded,
            0x0202 => ErrorCode::ElfArgumentBlobMalformed,
            0x0301 => ErrorCode::PortAccessDenied,
            0x0302 => ErrorCode::PortRequestInvalid,
            _ => ErrorCode::Unknown,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL_ERROR_CODES: [ErrorCode; 17] = [
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
        ErrorCode::ElfArgumentLimitExceeded,
        ErrorCode::ElfArgumentBlobMalformed,
        ErrorCode::PortAccessDenied,
        ErrorCode::PortRequestInvalid,
        ErrorCode::Unknown,
    ];

    #[test]
    fn every_error_code_variant_roundtrips_through_the_register_codec() {
        for code in ALL_ERROR_CODES {
            let raw = code.to_reg();
            assert_ne!(raw & ERROR_BIT, 0);
            assert!(!is_token(raw));
            assert_eq!(ErrorCode::from_code(raw), code);
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
    fn is_token_rejects_high_bit_garbage_and_exec_sentinel() {
        assert!(!is_token(1usize << 40));
        assert!(!is_token(u64::MAX as usize));
    }
}
