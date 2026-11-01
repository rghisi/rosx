pub fn fletcher32(data: &[u8]) -> u32 {
    let mut sum1: u32 = 0;
    let mut sum2: u32 = 0;
    let mut chunks = data.chunks_exact(2);
    for word in &mut chunks {
        sum1 = (sum1 + u16::from_le_bytes([word[0], word[1]]) as u32) % 65535;
        sum2 = (sum2 + sum1) % 65535;
    }
    if let Some(&tail) = chunks.remainder().first() {
        sum1 = (sum1 + tail as u32) % 65535;
        sum2 = (sum2 + sum1) % 65535;
    }
    (sum2 << 16) | sum1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_input_hashes_to_zero() {
        assert_eq!(fletcher32(&[]), 0);
    }

    #[test]
    fn pinned_vectors() {
        assert_eq!(fletcher32(b"a"), 0x0061_0061);
        assert_eq!(fletcher32(b"abcd"), 0x2926_C6C4);
        assert_eq!(fletcher32(b"abcdefgh"), 0xEBE1_9591);
        assert_eq!(fletcher32(&[0u8; 4096]), 0);
    }

    #[test]
    fn odd_tail_contributes() {
        assert_ne!(fletcher32(b"abcde"), fletcher32(b"abcd"));
        assert_ne!(fletcher32(b"abcde"), fletcher32(b"abcd\0"));
    }

    #[test]
    fn byte_order_matters() {
        assert_ne!(fletcher32(b"abcd"), fletcher32(b"badc"));
    }

    #[test]
    fn single_byte_change_changes_hash() {
        let mut data = [0x5Au8; 256];
        let before = fletcher32(&data);
        data[127] ^= 0x01;
        assert_ne!(fletcher32(&data), before);
    }

    #[test]
    fn sums_wrap_modulo_65535() {
        let data = [0xFFu8; 2 * 65_535];
        assert_eq!(fletcher32(&data), 0);
        let mut ramped = Vec::new();
        for _ in 0..20 {
            ramped.extend(0u8..=255);
        }
        assert_eq!(fletcher32(&ramped), 0x8255_FB04);
    }
}
