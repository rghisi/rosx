const CRC32_POLYNOMIAL_REFLECTED: u32 = 0xEDB8_8320;

pub fn crc32(data: &[u8]) -> u32 {
    let mut accumulator = 0xFFFF_FFFFu32;
    for byte in data {
        accumulator ^= u32::from(*byte);
        for _ in 0..8 {
            let mask = (accumulator & 1).wrapping_neg();
            accumulator = (accumulator >> 1) ^ (CRC32_POLYNOMIAL_REFLECTED & mask);
        }
    }
    !accumulator
}

#[cfg(test)]
mod tests {
    use super::crc32;

    #[test]
    fn empty_input_has_zero_checksum() {
        assert_eq!(crc32(&[]), 0x0000_0000);
    }

    #[test]
    fn check_field_matches_ieee_802_3_reference() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }

    #[test]
    fn pangram_matches_reference() {
        assert_eq!(crc32(b"The quick brown fox jumps over the lazy dog"), 0x414F_A339);
    }

    #[test]
    fn single_zero_byte_matches_reference() {
        assert_eq!(crc32(&[0x00]), 0xD202_EF8D);
    }
}
