#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Message {
    pub tag: u64,
    pub words: [u64; 4],
    pub payload_ptr: usize,
    pub payload_len: usize,
}

impl Message {
    pub fn new(tag: u64) -> Self {
        Self {
            tag,
            words: [0; 4],
            payload_ptr: 0,
            payload_len: 0,
        }
    }

    pub fn with_word(mut self, index: usize, value: u64) -> Self {
        self.words[index] = value;
        self
    }

    pub fn with_payload(mut self, ptr: usize, len: usize) -> Self {
        self.payload_ptr = ptr;
        self.payload_len = len;
        self
    }
}

pub mod endpoint {
    pub const TERMINAL: u32 = 1;
    pub const KEYBOARD: u32 = 2;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_message_has_correct_tag() {
        let msg = Message::new(7);
        assert_eq!(msg.tag, 7);
    }

    #[test]
    fn new_message_words_are_zero() {
        let msg = Message::new(0);
        assert_eq!(msg.words, [0u64; 4]);
    }

    #[test]
    fn new_message_has_no_payload() {
        let msg = Message::new(0);
        assert_eq!(msg.payload_ptr, 0);
        assert_eq!(msg.payload_len, 0);
    }

    #[test]
    fn with_word_sets_correct_slot() {
        let msg = Message::new(1).with_word(0, 42);
        assert_eq!(msg.words[0], 42);
        assert_eq!(msg.words[1], 0);
    }

    #[test]
    fn with_payload_sets_ptr_and_len() {
        let data = b"hello";
        let msg = Message::new(1).with_payload(data.as_ptr() as usize, data.len());
        assert_eq!(msg.payload_ptr, data.as_ptr() as usize);
        assert_eq!(msg.payload_len, 5);
    }
}
