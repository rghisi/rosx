use crate::pio;

const DEBUG_PORT: u16 = 0xE9;

const HEX_DIGITS: &[u8; 16] = b"0123456789abcdef";
const LARGEST_DECIMAL_DIGITS: usize = 20;

pub struct DebugConsole;

impl DebugConsole {
    fn write_byte(&self, byte: u8) {
        pio::outb(DEBUG_PORT, byte);
    }

    pub fn write_str(&self, text: &str) {
        for byte in text.bytes() {
            self.write_byte(byte);
        }
    }

    pub fn write_line(&self, text: &str) {
        self.write_str(text);
        self.write_str("\r\n");
    }

    pub fn write_hex(&self, value: u64, nibbles: u32) {
        render_hex(|byte| self.write_byte(byte), value, nibbles);
    }

    pub fn write_dec(&self, value: u64) {
        render_dec(|byte| self.write_byte(byte), value);
    }
}

impl core::fmt::Write for DebugConsole {
    fn write_str(&mut self, text: &str) -> core::fmt::Result {
        for byte in text.bytes() {
            self.write_byte(byte);
        }
        Ok(())
    }
}

pub(crate) fn render_hex(mut emit: impl FnMut(u8), value: u64, nibbles: u32) {
    emit(b'0');
    emit(b'x');
    for nibble_index in (0..nibbles).rev() {
        let digit = ((value >> (nibble_index * 4)) & 0xF) as usize;
        emit(HEX_DIGITS[digit]);
    }
}

pub(crate) fn render_dec(mut emit: impl FnMut(u8), value: u64) {
    let mut digits = [b'0'; LARGEST_DECIMAL_DIGITS];
    let mut first_digit = digits.len();
    let mut remaining = value;
    loop {
        first_digit -= 1;
        digits[first_digit] = b'0' + (remaining % 10) as u8;
        remaining /= 10;
        if remaining == 0 {
            break;
        }
    }
    match core::str::from_utf8(&digits[first_digit..]) {
        Ok(rendered) => {
            for byte in rendered.bytes() {
                emit(byte);
            }
        }
        Err(_) => emit(b'?'),
    }
}

#[cfg(test)]
mod tests {
    use super::{render_dec, render_hex};

    fn rendered_hex(value: u64, nibbles: u32) -> String {
        let mut bytes = Vec::new();
        render_hex(|byte| bytes.push(byte), value, nibbles);
        String::from_utf8(bytes).expect("rendered hex is ascii")
    }

    fn rendered_dec(value: u64) -> String {
        let mut bytes = Vec::new();
        render_dec(|byte| bytes.push(byte), value);
        String::from_utf8(bytes).expect("rendered decimal is ascii")
    }

    #[test]
    fn hex_carries_the_prefix_and_zero_padding() {
        assert_eq!(rendered_hex(0xABCD, 8), "0x0000abcd");
        assert_eq!(rendered_hex(0, 4), "0x0000");
    }

    #[test]
    fn hex_renders_lowercase_digits() {
        assert_eq!(rendered_hex(0xDEAD_BEEF, 8), "0xdeadbeef");
    }

    #[test]
    fn hex_reaches_the_full_u64_width() {
        assert_eq!(rendered_hex(u64::MAX, 16), "0xffffffffffffffff");
    }

    #[test]
    fn decimal_has_no_padding_and_zero_is_one_digit() {
        assert_eq!(rendered_dec(0), "0");
        assert_eq!(rendered_dec(9), "9");
        assert_eq!(rendered_dec(12_345), "12345");
    }

    #[test]
    fn decimal_reaches_the_full_u64_width() {
        assert_eq!(rendered_dec(u64::MAX), "18446744073709551615");
    }
}
