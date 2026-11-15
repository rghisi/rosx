pub const NULL: u64 = 0;

const CODE_ACCESS: u8 = 0x9A;
const DATA_ACCESS: u8 = 0x92;
const FLAGS_16: u8 = 0b1000;
const FLAGS_32: u8 = 0b1100;
const FULL_LIMIT: u32 = 0x000F_FFFF;

pub const fn encode(base: u32, limit: u32, access: u8, flags: u8) -> u64 {
    (limit as u64 & 0xFFFF)
        | ((base as u64 & 0x00FF_FFFF) << 16)
        | ((access as u64) << 40)
        | ((flags as u64 & 0x0F) << 52)
        | ((limit as u64 >> 16) & 0x0F) << 48
        | ((base as u64 >> 24) << 56)
}

pub const fn code16() -> u64 {
    encode(0, FULL_LIMIT, CODE_ACCESS, FLAGS_16)
}

pub const fn data16() -> u64 {
    encode(0, FULL_LIMIT, DATA_ACCESS, FLAGS_16)
}

pub const fn code32() -> u64 {
    encode(0, FULL_LIMIT, CODE_ACCESS, FLAGS_32)
}

pub const fn data32() -> u64 {
    encode(0, FULL_LIMIT, DATA_ACCESS, FLAGS_32)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn access(descriptor: u64) -> u8 {
        (descriptor >> 40) as u8
    }

    fn flags(descriptor: u64) -> u8 {
        (descriptor >> 52) as u8 & 0x0F
    }

    fn limit(descriptor: u64) -> u32 {
        (descriptor as u32 & 0xFFFF) | ((descriptor >> 32) as u32 & 0x000F_0000)
    }

    fn base(descriptor: u64) -> u32 {
        ((descriptor >> 16) as u32 & 0x00FF_FFFF) | ((descriptor >> 32) as u32 & 0xFF00_0000)
    }

    #[test]
    fn flat_code32_matches_the_canonical_encoding() {
        assert_eq!(code32(), 0x00CF_9A00_0000_FFFF);
    }

    #[test]
    fn flat_data32_matches_the_canonical_encoding() {
        assert_eq!(data32(), 0x00CF_9200_0000_FFFF);
    }

    #[test]
    fn thunk_code16_matches_the_canonical_encoding() {
        assert_eq!(code16(), 0x008F_9A00_0000_FFFF);
    }

    #[test]
    fn thunk_data16_matches_the_canonical_encoding() {
        assert_eq!(data16(), 0x008F_9200_0000_FFFF);
    }

    #[test]
    fn null_selector_is_zero() {
        assert_eq!(NULL, 0);
    }

    #[test]
    fn encode_splits_base_and_limit_across_their_descriptor_fields() {
        let descriptor = encode(0xDEAD_BEEF, 0x000A_BCDE, CODE_ACCESS, FLAGS_32);
        assert_eq!(base(descriptor), 0xDEAD_BEEF);
        assert_eq!(limit(descriptor), 0x000A_BCDE);
        assert_eq!(access(descriptor), CODE_ACCESS);
        assert_eq!(flags(descriptor), FLAGS_32);
    }

    #[test]
    fn encode_keeps_base_and_limit_fields_disjoint() {
        let only_base = encode(0xFFFF_FFFF, 0, 0, 0);
        assert_eq!(base(only_base), 0xFFFF_FFFF);
        assert_eq!(limit(only_base), 0);
        let only_limit = encode(0, 0x000F_FFFF, 0, 0);
        assert_eq!(limit(only_limit), 0x000F_FFFF);
        assert_eq!(base(only_limit), 0);
    }
}
