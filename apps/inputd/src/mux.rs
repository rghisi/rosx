use alloc::collections::VecDeque;
use input_protocol::wire;
use system::ipc::MESSAGE_PAYLOAD_BYTES;

pub const BUFFERED_CHAR_CAP: usize = 32;

#[derive(Debug, PartialEq, Eq)]
pub enum MuxAction<Reader> {
    Reply(Reader, [u8; MESSAGE_PAYLOAD_BYTES]),
}

pub struct InputMux<Reader> {
    pending_reads: VecDeque<Reader>,
    buffered_chars: VecDeque<char>,
}

impl<Reader: Copy + PartialEq> InputMux<Reader> {
    pub fn new() -> InputMux<Reader> {
        InputMux {
            pending_reads: VecDeque::new(),
            buffered_chars: VecDeque::new(),
        }
    }

    pub fn handle_read(&mut self, reader: Reader) -> Option<MuxAction<Reader>> {
        match self.buffered_chars.pop_front() {
            Some(c) => Some(MuxAction::Reply(reader, wire::encode_key(c))),
            None => {
                self.pending_reads.push_back(reader);
                None
            }
        }
    }

    pub fn handle_read_nowait(&mut self, reader: Reader) -> Option<MuxAction<Reader>> {
        match self.buffered_chars.pop_front() {
            Some(c) => Some(MuxAction::Reply(reader, wire::encode_key(c))),
            None => Some(MuxAction::Reply(reader, wire::encode_no_key())),
        }
    }

    pub fn handle_push(&mut self, c: char) -> Option<MuxAction<Reader>> {
        match self.pending_reads.pop_front() {
            Some(reader) => Some(MuxAction::Reply(reader, wire::encode_key(c))),
            None => {
                if self.buffered_chars.len() == BUFFERED_CHAR_CAP {
                    self.buffered_chars.pop_front();
                }
                self.buffered_chars.push_back(c);
                None
            }
        }
    }

    pub fn forget(&mut self, reader: Reader) {
        self.pending_reads.retain(|pending| *pending != reader);
    }
}

impl<Reader: Copy + PartialEq> Default for InputMux<Reader> {
    fn default() -> InputMux<Reader> {
        InputMux::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use input_protocol::wire::{frame_char, frame_tag, TAG_KEY, TAG_NO_KEY};

    fn replied_char(frame: &[u8; MESSAGE_PAYLOAD_BYTES]) -> char {
        assert_eq!(frame_tag(frame), TAG_KEY);
        frame_char(frame).expect("char frame carries a char")
    }

    #[test]
    fn read_then_push_replies_key_to_the_waiting_reader() {
        let mut mux: InputMux<u32> = InputMux::new();
        assert_eq!(mux.handle_read(7), None);
        let action = mux.handle_push('a').expect("pending reader receives the char");
        let MuxAction::Reply(reader, frame) = action else {
            panic!("expected a reply");
        };
        assert_eq!(reader, 7);
        assert_eq!(replied_char(&frame), 'a');
    }

    #[test]
    fn push_then_read_replies_from_the_buffer() {
        let mut mux: InputMux<u32> = InputMux::new();
        assert_eq!(mux.handle_push('b'), None);
        let action = mux.handle_read(3).expect("buffered char answers the read");
        let MuxAction::Reply(reader, frame) = action else {
            panic!("expected a reply");
        };
        assert_eq!(reader, 3);
        assert_eq!(replied_char(&frame), 'b');
    }

    #[test]
    fn nowait_replies_key_when_the_buffer_is_non_empty() {
        let mut mux: InputMux<u32> = InputMux::new();
        mux.handle_push('c');
        let action = mux.handle_read_nowait(5).expect("nowait always answers");
        let MuxAction::Reply(reader, frame) = action else {
            panic!("expected a reply");
        };
        assert_eq!(reader, 5);
        assert_eq!(replied_char(&frame), 'c');
    }

    #[test]
    fn nowait_replies_no_key_when_the_buffer_is_empty() {
        let mut mux: InputMux<u32> = InputMux::new();
        let action = mux.handle_read_nowait(9).expect("nowait always answers");
        let MuxAction::Reply(reader, frame) = action else {
            panic!("expected a reply");
        };
        assert_eq!(reader, 9);
        assert_eq!(frame_tag(&frame), TAG_NO_KEY);
    }

    #[test]
    fn two_readers_receive_one_char_each_in_arrival_order() {
        let mut mux: InputMux<u32> = InputMux::new();
        assert_eq!(mux.handle_read(1), None);
        assert_eq!(mux.handle_read(2), None);
        let MuxAction::Reply(first_reader, first) = mux.handle_push('x').expect("first push") else {
            panic!("expected a reply");
        };
        let MuxAction::Reply(second_reader, second) =
            mux.handle_push('y').expect("second push")
        else {
            panic!("expected a reply");
        };
        assert_eq!(first_reader, 1);
        assert_eq!(replied_char(&first), 'x');
        assert_eq!(second_reader, 2);
        assert_eq!(replied_char(&second), 'y');
    }

    #[test]
    fn one_char_is_delivered_to_exactly_one_reader_never_broadcast() {
        let mut mux: InputMux<u32> = InputMux::new();
        mux.handle_read(1);
        mux.handle_read(2);
        let MuxAction::Reply(first, frame) = mux.handle_push('m').expect("first reader served")
        else {
            panic!("expected a reply");
        };
        assert_eq!(first, 1);
        assert_eq!(replied_char(&frame), 'm');
        let MuxAction::Reply(_, probe) = mux.handle_read_nowait(3).expect("nowait answers") else {
            panic!("expected a reply");
        };
        assert_eq!(frame_tag(&probe), TAG_NO_KEY);
    }

    #[test]
    fn overflow_drops_the_oldest_char_at_capacity() {
        let mut mux: InputMux<u32> = InputMux::new();
        for i in 0..BUFFERED_CHAR_CAP {
            let oldest = char::from(b'a' + i as u8);
            assert_eq!(mux.handle_push(oldest), None);
        }
        assert_eq!(mux.handle_push('Z'), None);
        let action = mux.handle_read(1).expect("read after overflow");
        let MuxAction::Reply(_, frame) = action else {
            panic!("expected a reply");
        };
        assert_eq!(replied_char(&frame), 'b');
    }

    #[test]
    fn forget_evicts_a_dead_reader_and_the_next_push_skips_it() {
        let mut mux: InputMux<u32> = InputMux::new();
        mux.handle_read(1);
        mux.forget(1);
        mux.handle_read(2);
        let action = mux.handle_push('k').expect("live reader receives the char");
        let MuxAction::Reply(reader, frame) = action else {
            panic!("expected a reply");
        };
        assert_eq!(reader, 2);
        assert_eq!(replied_char(&frame), 'k');
    }

    #[test]
    fn malformed_push_frame_leaves_state_unchanged() {
        let mut mux: InputMux<u32> = InputMux::new();
        let mut malformed = wire::encode_push('a');
        malformed[1..5].copy_from_slice(&0xD800u32.to_le_bytes());
        if let Some(c) = frame_char(&malformed) {
            mux.handle_push(c);
        }
        let action = mux.handle_read_nowait(1).expect("nowait always answers");
        let MuxAction::Reply(_, frame) = action else {
            panic!("expected a reply");
        };
        assert_eq!(frame_tag(&frame), TAG_NO_KEY);
    }

    #[test]
    fn push_without_a_reader_buffers_and_never_registers_the_pusher() {
        let mut mux: InputMux<u32> = InputMux::new();
        assert_eq!(mux.handle_push('1'), None);
        assert_eq!(mux.handle_push('2'), None);
        let first = mux.handle_read(10).expect("first buffered char");
        let second = mux.handle_read(11).expect("second buffered char");
        let MuxAction::Reply(first_reader, first_frame) = first else {
            panic!("expected a reply");
        };
        let MuxAction::Reply(second_reader, second_frame) = second else {
            panic!("expected a reply");
        };
        assert_eq!(first_reader, 10);
        assert_eq!(replied_char(&first_frame), '1');
        assert_eq!(second_reader, 11);
        assert_eq!(replied_char(&second_frame), '2');
    }
}
