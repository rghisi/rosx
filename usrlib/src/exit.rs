use system::exit::{
    decode_code, decode_reason, EXIT_REASON_CLEAN, EXIT_REASON_INTERRUPTED, EXIT_REASON_PANICKED,
};

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum ExitStatus {
    Clean(u32),
    Panicked,
    Interrupted,
    Unknown { reason: u32, code: u32 },
}

impl From<i32> for ExitStatus {
    fn from(status: i32) -> Self {
        match decode_reason(status) {
            EXIT_REASON_CLEAN => ExitStatus::Clean(decode_code(status)),
            EXIT_REASON_PANICKED => ExitStatus::Panicked,
            EXIT_REASON_INTERRUPTED => ExitStatus::Interrupted,
            reason => ExitStatus::Unknown { reason, code: decode_code(status) },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use system::exit::{clean_status, encode, panicked_status, EXIT_CODE_PANIC};

    #[test]
    fn zero_decodes_to_clean_zero() {
        assert_eq!(ExitStatus::from(0), ExitStatus::Clean(0));
    }

    #[test]
    fn clean_status_decodes_to_its_code() {
        assert_eq!(ExitStatus::from(clean_status(3).unwrap()), ExitStatus::Clean(3));
        assert_eq!(
            ExitStatus::from(clean_status(system::exit::MAX_EXIT_CODE).unwrap()),
            ExitStatus::Clean(system::exit::MAX_EXIT_CODE)
        );
    }

    #[test]
    fn panicked_status_decodes_to_panicked() {
        assert_eq!(ExitStatus::from(panicked_status()), ExitStatus::Panicked);
    }

    #[test]
    fn interrupted_status_decodes_to_interrupted() {
        assert_eq!(
            ExitStatus::from(system::exit::interrupted_status()),
            ExitStatus::Interrupted
        );
    }

    #[test]
    fn reserved_reason_decodes_to_unknown_carrying_its_fields() {
        let status = encode(3, 5);
        assert_eq!(ExitStatus::from(status), ExitStatus::Unknown { reason: 3, code: 5 });
    }

    #[test]
    fn the_panic_request_code_itself_is_not_the_panicked_view() {
        assert_eq!(
            ExitStatus::from(EXIT_CODE_PANIC as i32),
            ExitStatus::Clean(EXIT_CODE_PANIC)
        );
    }
}
