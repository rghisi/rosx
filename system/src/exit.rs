pub const EXIT_REASON_CLEAN: u32 = 0;
pub const EXIT_REASON_PANICKED: u32 = 1;

pub const MAX_EXIT_CODE: u32 = 0x00FF_FFFF;

pub const EXIT_CODE_PANIC: u32 = 0x000A_121C;

pub const fn encode(reason: u32, code: u32) -> i32 {
    ((reason << 24) | (code & MAX_EXIT_CODE)) as i32
}

pub const fn clean_status(code: u32) -> Option<i32> {
    if code > MAX_EXIT_CODE {
        None
    } else {
        Some(encode(EXIT_REASON_CLEAN, code))
    }
}

pub const fn panicked_status() -> i32 {
    encode(EXIT_REASON_PANICKED, 0)
}

pub const fn decode_reason(status: i32) -> u32 {
    (status as u32) >> 24
}

pub const fn decode_code(status: i32) -> u32 {
    (status as u32) & MAX_EXIT_CODE
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_zero_status_is_the_zero_value() {
        assert_eq!(clean_status(0), Some(0));
        assert_eq!(decode_reason(0), EXIT_REASON_CLEAN);
        assert_eq!(decode_code(0), 0);
    }

    #[test]
    fn clean_status_carries_only_its_code_in_the_low_field() {
        let status = clean_status(3).unwrap();
        assert_eq!(status, 3);
        assert_eq!(decode_reason(status), EXIT_REASON_CLEAN);
        assert_eq!(decode_code(status), 3);
    }

    #[test]
    fn clean_status_accepts_the_field_maximum() {
        let status = clean_status(MAX_EXIT_CODE).unwrap();
        assert_eq!(decode_reason(status), EXIT_REASON_CLEAN);
        assert_eq!(decode_code(status), MAX_EXIT_CODE);
    }

    #[test]
    fn clean_status_rejects_codes_beyond_the_24_bit_field() {
        assert_eq!(clean_status(MAX_EXIT_CODE + 1), None);
        assert_eq!(clean_status(u32::MAX), None);
    }

    #[test]
    fn panicked_status_carries_the_panicked_reason_and_no_code() {
        let status = panicked_status();
        assert_eq!(decode_reason(status), EXIT_REASON_PANICKED);
        assert_eq!(decode_code(status), 0);
    }

    #[test]
    fn reason_and_code_round_trip_through_the_codec() {
        for code in [0u32, 1, 42, 1023, MAX_EXIT_CODE] {
            let status = encode(EXIT_REASON_PANICKED, code);
            assert_eq!(decode_reason(status), EXIT_REASON_PANICKED);
            assert_eq!(decode_code(status), code);
        }
    }

    #[test]
    fn reserved_reasons_survive_decoding_instead_of_reading_as_clean() {
        let status = encode(2, 5);
        assert_eq!(decode_reason(status), 2);
        assert_eq!(decode_code(status), 5);
    }

    #[test]
    fn known_reasons_stay_inside_the_low_reason_band() {
        assert!(encode(EXIT_REASON_CLEAN, MAX_EXIT_CODE) >= 0);
        assert!(encode(EXIT_REASON_PANICKED, MAX_EXIT_CODE) >= 0);
    }

    #[test]
    fn panic_exit_code_lives_inside_the_code_field() {
        assert!(EXIT_CODE_PANIC <= MAX_EXIT_CODE);
        assert_eq!(clean_status(EXIT_CODE_PANIC), Some(EXIT_CODE_PANIC as i32));
    }

    #[test]
    fn panic_exit_code_is_far_from_plausible_app_codes() {
        assert!(EXIT_CODE_PANIC > 0xFFFF);
        assert_ne!(EXIT_CODE_PANIC, MAX_EXIT_CODE);
    }
}
