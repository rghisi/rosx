use system::ipc::MESSAGE_PAYLOAD_BYTES;

pub const SERVICE_NAME: &str = "INPUTD";
pub const SOURCE_NAME_PS2KEYBOARD: &str = "PS2KEYBOARD";

pub const TAG_READ: u8 = 1;
pub const TAG_READ_NOWAIT: u8 = 2;
pub const TAG_PUSH: u8 = 3;
pub const TAG_KEY: u8 = 4;
pub const TAG_NO_KEY: u8 = 5;

pub fn encode_read() -> [u8; MESSAGE_PAYLOAD_BYTES] {
    encode_tag_only(TAG_READ)
}

pub fn encode_read_nowait() -> [u8; MESSAGE_PAYLOAD_BYTES] {
    encode_tag_only(TAG_READ_NOWAIT)
}

pub fn encode_no_key() -> [u8; MESSAGE_PAYLOAD_BYTES] {
    encode_tag_only(TAG_NO_KEY)
}

pub fn encode_push(c: char) -> [u8; MESSAGE_PAYLOAD_BYTES] {
    encode_char_frame(TAG_PUSH, c)
}

pub fn encode_key(c: char) -> [u8; MESSAGE_PAYLOAD_BYTES] {
    encode_char_frame(TAG_KEY, c)
}

pub fn encode_scancode(scancode: u8) -> [u8; MESSAGE_PAYLOAD_BYTES] {
    let mut data = [0u8; MESSAGE_PAYLOAD_BYTES];
    data[0] = scancode;
    data
}

pub fn frame_tag(data: &[u8; MESSAGE_PAYLOAD_BYTES]) -> u8 {
    data[0]
}

pub fn frame_char(data: &[u8; MESSAGE_PAYLOAD_BYTES]) -> Option<char> {
    let word = u32::from_le_bytes([data[1], data[2], data[3], data[4]]);
    char::from_u32(word)
}

pub fn scancode_of(data: &[u8; MESSAGE_PAYLOAD_BYTES]) -> u8 {
    data[0]
}

fn encode_tag_only(tag: u8) -> [u8; MESSAGE_PAYLOAD_BYTES] {
    let mut data = [0u8; MESSAGE_PAYLOAD_BYTES];
    data[0] = tag;
    data
}

fn encode_char_frame(tag: u8, c: char) -> [u8; MESSAGE_PAYLOAD_BYTES] {
    let mut data = [0u8; MESSAGE_PAYLOAD_BYTES];
    data[0] = tag;
    data[1..5].copy_from_slice(&(c as u32).to_le_bytes());
    data
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nonzero_bytes(data: &[u8; MESSAGE_PAYLOAD_BYTES]) -> Vec<u8> {
        data.iter().copied().filter(|b| *b != 0).collect()
    }

    #[test]
    fn each_encoder_places_its_tag_at_index_zero() {
        assert_eq!(frame_tag(&encode_read()), TAG_READ);
        assert_eq!(frame_tag(&encode_read_nowait()), TAG_READ_NOWAIT);
        assert_eq!(frame_tag(&encode_no_key()), TAG_NO_KEY);
        assert_eq!(frame_tag(&encode_push('a')), TAG_PUSH);
        assert_eq!(frame_tag(&encode_key('a')), TAG_KEY);
    }

    #[test]
    fn tag_only_encoders_emit_exactly_one_tag_and_zero_padding() {
        for data in [encode_read(), encode_read_nowait(), encode_no_key()] {
            assert_eq!(nonzero_bytes(&data), vec![data[0]]);
        }
        assert_eq!(
            [encode_read(), encode_read_nowait(), encode_no_key()].map(|d| frame_tag(&d)),
            [TAG_READ, TAG_READ_NOWAIT, TAG_NO_KEY]
        );
    }

    #[test]
    fn char_frames_round_trip_through_frame_char() {
        for c in ['a', 'Z', '0', ' ', '~', '\u{1F980}'] {
            assert_eq!(frame_char(&encode_push(c)), Some(c));
            assert_eq!(frame_char(&encode_key(c)), Some(c));
        }
    }

    #[test]
    fn char_frames_place_the_codepoint_little_endian_after_the_tag() {
        let data = encode_key('A');
        assert_eq!(data[0], TAG_KEY);
        assert_eq!(data[1..5], ('A' as u32).to_le_bytes());
        assert!(data[5..].iter().all(|b| *b == 0));
    }

    #[test]
    fn frame_char_rejects_non_char_words() {
        let mut data = encode_key('a');
        data[1..5].copy_from_slice(&0xD800u32.to_le_bytes());
        assert_eq!(frame_char(&data), None);
    }

    #[test]
    fn push_and_key_frames_differ_only_by_tag() {
        let push = encode_push('x');
        let key = encode_key('x');
        assert_eq!(&push[1..], &key[1..]);
        assert_eq!(push[0], TAG_PUSH);
        assert_eq!(key[0], TAG_KEY);
    }

    #[test]
    fn scancode_frames_round_trip_and_zero_the_tail() {
        for scancode in [0x00u8, 0x1C, 0x2A, 0x80, 0xE0, 0xFF] {
            let data = encode_scancode(scancode);
            assert_eq!(scancode_of(&data), scancode);
            assert!(data[1..].iter().all(|b| *b == 0));
        }
    }
}
