pub const RPC_MAX_DATA_BYTES: usize = 64 * 1024;
pub const MAX_NAME_BYTES: usize = 57;
pub const SERVICE_NAME: &str = "VFS";

pub const OPEN: u8 = 1;
pub const READ: u8 = 2;
pub const WRITE: u8 = 3;
pub const CLOSE: u8 = 4;
pub const DELETE: u8 = 5;
pub const LIST: u8 = 6;

pub const STATUS_OK: u8 = 0;
pub const STATUS_ERR: u8 = 1;
pub const STATUS_EOF: u8 = 2;

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[repr(u8)]
pub enum VfsError {
    NotFound = 1,
    AlreadyExists = 2,
    NotPermitted = 3,
    BadHandle = 4,
    InvalidPath = 5,
    ListTooLarge = 6,
    ServerResource = 7,
    Protocol,
}

impl VfsError {
    pub fn from_byte(byte: u8) -> VfsError {
        match byte {
            1 => VfsError::NotFound,
            2 => VfsError::AlreadyExists,
            3 => VfsError::NotPermitted,
            4 => VfsError::BadHandle,
            5 => VfsError::InvalidPath,
            6 => VfsError::ListTooLarge,
            7 => VfsError::ServerResource,
            _ => VfsError::Protocol,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Request<'a> {
    Open {
        mode: u8,
        name: &'a [u8],
    },
    Read {
        handle: u32,
        offset: u64,
        count: u32,
    },
    Write {
        handle: u32,
        offset: u64,
        token: u32,
    },
    Close {
        handle: u32,
    },
    Delete {
        name: &'a [u8],
    },
    List,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Reply {
    Ack,
    Opened { handle: u32 },
    ReadData { token: u32, count: u32 },
    Written { bytes: u32 },
    Listed { token: u32, total: u32 },
    Eof,
    Err(VfsError),
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum Expect {
    Ack,
    Opened,
    ReadData,
    Written,
    Listed,
}

pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_NAME_BYTES
        && !name.as_bytes().contains(&b'\n')
        && !name.as_bytes().contains(&0u8)
}

pub fn encode_open(mode: u8, name: &str) -> Option<[u8; 60]> {
    if !valid_name(name) {
        return None;
    }
    let mut frame = [0u8; 60];
    frame[0] = OPEN;
    frame[1] = mode;
    frame[2] = name.len() as u8;
    frame[3..3 + name.len()].copy_from_slice(name.as_bytes());
    Some(frame)
}

pub fn encode_read(handle: u32, offset: u64, count: u32) -> [u8; 60] {
    let mut frame = [0u8; 60];
    frame[0] = READ;
    frame[1..5].copy_from_slice(&handle.to_le_bytes());
    frame[5..13].copy_from_slice(&offset.to_le_bytes());
    frame[13..17].copy_from_slice(&count.to_le_bytes());
    frame
}

pub fn encode_write(handle: u32, offset: u64, token: u32) -> [u8; 60] {
    let mut frame = [0u8; 60];
    frame[0] = WRITE;
    frame[1..5].copy_from_slice(&handle.to_le_bytes());
    frame[5..13].copy_from_slice(&offset.to_le_bytes());
    frame[13..17].copy_from_slice(&token.to_le_bytes());
    frame
}

pub fn encode_close(handle: u32) -> [u8; 60] {
    let mut frame = [0u8; 60];
    frame[0] = CLOSE;
    frame[1..5].copy_from_slice(&handle.to_le_bytes());
    frame
}

pub fn encode_delete(name: &str) -> Option<[u8; 60]> {
    if !valid_name(name) {
        return None;
    }
    let mut frame = [0u8; 60];
    frame[0] = DELETE;
    frame[1] = name.len() as u8;
    frame[2..2 + name.len()].copy_from_slice(name.as_bytes());
    Some(frame)
}

pub fn encode_list() -> [u8; 60] {
    let mut frame = [0u8; 60];
    frame[0] = LIST;
    frame
}

pub fn decode_request(frame: &[u8; 60]) -> Option<Request<'_>> {
    match frame[0] {
        OPEN => Some(Request::Open {
            mode: frame[1],
            name: decode_name(frame, 2)?,
        }),
        READ => Some(Request::Read {
            handle: read_u32(frame, 1),
            offset: read_u64(frame, 5),
            count: read_u32(frame, 13),
        }),
        WRITE => Some(Request::Write {
            handle: read_u32(frame, 1),
            offset: read_u64(frame, 5),
            token: read_u32(frame, 13),
        }),
        CLOSE => Some(Request::Close {
            handle: read_u32(frame, 1),
        }),
        DELETE => Some(Request::Delete {
            name: decode_name(frame, 1)?,
        }),
        LIST => Some(Request::List),
        _ => None,
    }
}

fn decode_name(frame: &[u8; 60], length_at: usize) -> Option<&[u8]> {
    let name_len = frame[length_at] as usize;
    if name_len == 0 || name_len > MAX_NAME_BYTES {
        return None;
    }
    let end = length_at + 1 + name_len;
    if end > frame.len() {
        return None;
    }
    Some(&frame[length_at + 1..end])
}

pub fn decode_reply(frame: &[u8; 60], expect: Expect) -> Result<Reply, ()> {
    match frame[0] {
        STATUS_EOF => Ok(Reply::Eof),
        STATUS_ERR => Ok(Reply::Err(VfsError::from_byte(frame[1]))),
        STATUS_OK => match expect {
            Expect::Ack => {
                if frame[1..].iter().all(|byte| *byte == 0) {
                    Ok(Reply::Ack)
                } else {
                    Err(())
                }
            }
            Expect::Opened => {
                let handle = read_u32(frame, 1);
                if handle >= 1 {
                    Ok(Reply::Opened { handle })
                } else {
                    Err(())
                }
            }
            Expect::ReadData => {
                let token = read_u32(frame, 1);
                let count = read_u32(frame, 5);
                if count >= 1 {
                    Ok(Reply::ReadData { token, count })
                } else {
                    Err(())
                }
            }
            Expect::Written => Ok(Reply::Written {
                bytes: read_u32(frame, 1),
            }),
            Expect::Listed => {
                let token = read_u32(frame, 1);
                let total = read_u32(frame, 5);
                if total >= 1 {
                    Ok(Reply::Listed { token, total })
                } else {
                    Err(())
                }
            }
        },
        _ => Err(()),
    }
}

fn read_u32(frame: &[u8; 60], at: usize) -> u32 {
    u32::from_le_bytes(frame[at..at + 4].try_into().unwrap())
}

fn read_u64(frame: &[u8; 60], at: usize) -> u64 {
    u64::from_le_bytes(frame[at..at + 8].try_into().unwrap())
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL_EXPECTS: [Expect; 5] = [
        Expect::Ack,
        Expect::Opened,
        Expect::ReadData,
        Expect::Written,
        Expect::Listed,
    ];

    const WIRE_ERRORS: [(u8, VfsError); 7] = [
        (1, VfsError::NotFound),
        (2, VfsError::AlreadyExists),
        (3, VfsError::NotPermitted),
        (4, VfsError::BadHandle),
        (5, VfsError::InvalidPath),
        (6, VfsError::ListTooLarge),
        (7, VfsError::ServerResource),
    ];

    #[test]
    fn contract_constants_match_the_wire_table() {
        assert_eq!(RPC_MAX_DATA_BYTES, 65_536);
        assert_eq!(MAX_NAME_BYTES, 57);
        assert_eq!(SERVICE_NAME, "VFS");
        assert_eq!([OPEN, READ, WRITE, CLOSE, DELETE, LIST], [1, 2, 3, 4, 5, 6]);
        assert_eq!([STATUS_OK, STATUS_ERR, STATUS_EOF], [0, 1, 2]);
    }

    #[test]
    fn open_request_frame_pins_exact_byte_offsets() {
        let frame = encode_open(3, "a.txt").expect("valid name encodes");
        assert_eq!(frame[0], OPEN);
        assert_eq!(frame[1], 3);
        assert_eq!(frame[2], 5);
        assert_eq!(&frame[3..8], b"a.txt");
        assert!(frame[8..].iter().all(|byte| *byte == 0));
    }

    #[test]
    fn read_request_frame_pins_exact_byte_offsets() {
        let frame = encode_read(0x0102_0304, 0x0807_0605_0403_0201, 0x0001_0000);
        assert_eq!(frame[0], READ);
        assert_eq!(&frame[1..5], &[0x04u8, 0x03, 0x02, 0x01]);
        assert_eq!(
            &frame[5..13],
            &[0x01u8, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08]
        );
        assert_eq!(&frame[13..17], &[0x00u8, 0x00, 0x01, 0x00]);
        assert!(frame[17..].iter().all(|byte| *byte == 0));
    }

    #[test]
    fn write_request_frame_pins_exact_byte_offsets() {
        let frame = encode_write(0x0A0B_0C0D, 0x0E0F_1011_1213_1415, 0x1617_1819);
        assert_eq!(frame[0], WRITE);
        assert_eq!(&frame[1..5], &[0x0Du8, 0x0C, 0x0B, 0x0A]);
        assert_eq!(
            &frame[5..13],
            &[0x15u8, 0x14, 0x13, 0x12, 0x11, 0x10, 0x0F, 0x0E]
        );
        assert_eq!(&frame[13..17], &[0x19u8, 0x18, 0x17, 0x16]);
        assert!(frame[17..].iter().all(|byte| *byte == 0));
    }

    #[test]
    fn close_request_frame_pins_exact_byte_offsets() {
        let frame = encode_close(0x0102_0304);
        assert_eq!(frame[0], CLOSE);
        assert_eq!(&frame[1..5], &[0x04u8, 0x03, 0x02, 0x01]);
        assert!(frame[5..].iter().all(|byte| *byte == 0));
    }

    #[test]
    fn delete_request_frame_pins_exact_byte_offsets() {
        let frame = encode_delete("hi").expect("valid name encodes");
        assert_eq!(frame[0], DELETE);
        assert_eq!(frame[1], 2);
        assert_eq!(&frame[2..4], b"hi");
        assert!(frame[4..].iter().all(|byte| *byte == 0));
    }

    #[test]
    fn list_request_frame_pins_exact_byte_offsets() {
        let frame = encode_list();
        assert_eq!(frame[0], LIST);
        assert!(frame[1..].iter().all(|byte| *byte == 0));
    }

    #[test]
    fn open_roundtrips_through_decode_request() {
        let frame = encode_open(3, "notes.txt").expect("valid name encodes");
        assert_eq!(
            decode_request(&frame),
            Some(Request::Open {
                mode: 3,
                name: b"notes.txt"
            })
        );
    }

    #[test]
    fn read_roundtrips_through_decode_request() {
        let frame = encode_read(0x0102_0304, 0x0807_0605_0403_0201, 0x0001_0000);
        assert_eq!(
            decode_request(&frame),
            Some(Request::Read {
                handle: 0x0102_0304,
                offset: 0x0807_0605_0403_0201,
                count: 0x0001_0000,
            })
        );
    }

    #[test]
    fn write_roundtrips_through_decode_request() {
        let frame = encode_write(0x0A0B_0C0D, 0x0E0F_1011_1213_1415, 0x1617_1819);
        assert_eq!(
            decode_request(&frame),
            Some(Request::Write {
                handle: 0x0A0B_0C0D,
                offset: 0x0E0F_1011_1213_1415,
                token: 0x1617_1819,
            })
        );
    }

    #[test]
    fn close_roundtrips_through_decode_request() {
        let frame = encode_close(0x0102_0304);
        assert_eq!(
            decode_request(&frame),
            Some(Request::Close {
                handle: 0x0102_0304
            })
        );
    }

    #[test]
    fn delete_roundtrips_through_decode_request() {
        let frame = encode_delete("blob.bin").expect("valid name encodes");
        assert_eq!(
            decode_request(&frame),
            Some(Request::Delete { name: b"blob.bin" })
        );
    }

    #[test]
    fn list_roundtrips_through_decode_request() {
        let frame = encode_list();
        assert_eq!(decode_request(&frame), Some(Request::List));
    }

    #[test]
    fn boundary_name_of_fifty_seven_bytes_roundtrips_for_open_and_delete() {
        let name = "x".repeat(MAX_NAME_BYTES);
        let open = encode_open(1, &name).expect("max length name is valid");
        assert_eq!(open[2], 57);
        assert_eq!(&open[3..60], name.as_bytes());
        assert_eq!(
            decode_request(&open),
            Some(Request::Open {
                mode: 1,
                name: name.as_bytes()
            })
        );
        let delete = encode_delete(&name).expect("max length name is valid");
        assert_eq!(delete[1], 57);
        assert_eq!(&delete[2..59], name.as_bytes());
        assert!(delete[59..].iter().all(|byte| *byte == 0));
        assert_eq!(
            decode_request(&delete),
            Some(Request::Delete {
                name: name.as_bytes()
            })
        );
    }

    #[test]
    fn decode_request_rejects_unknown_op() {
        for op in [0u8, 7, 255] {
            let mut frame = [0u8; 60];
            frame[0] = op;
            assert_eq!(decode_request(&frame), None);
        }
    }

    #[test]
    fn decode_request_rejects_zero_name_len() {
        let mut open = [0u8; 60];
        open[0] = OPEN;
        open[1] = 1;
        assert_eq!(decode_request(&open), None);
        let mut delete = [0u8; 60];
        delete[0] = DELETE;
        assert_eq!(decode_request(&delete), None);
    }

    #[test]
    fn decode_request_rejects_name_len_above_max() {
        for length in [58u8, 59, 60, 255] {
            let mut open = [0u8; 60];
            open[0] = OPEN;
            open[1] = 1;
            open[2] = length;
            assert_eq!(decode_request(&open), None);
            let mut delete = [0u8; 60];
            delete[0] = DELETE;
            delete[1] = length;
            assert_eq!(decode_request(&delete), None);
        }
    }

    #[test]
    fn decode_request_rejects_declared_name_past_frame_end() {
        let mut delete = [0u8; 60];
        delete[0] = DELETE;
        delete[1] = 59;
        assert_eq!(decode_request(&delete), None);
        let mut open = [0u8; 60];
        open[0] = OPEN;
        open[2] = 58;
        assert_eq!(decode_request(&open), None);
    }

    #[test]
    fn valid_name_accepts_one_and_fifty_seven_bytes() {
        assert!(valid_name("a"));
        assert!(valid_name("a.txt"));
        assert!(valid_name(&"x".repeat(MAX_NAME_BYTES)));
    }

    #[test]
    fn valid_name_rejects_empty_and_overlong() {
        assert!(!valid_name(""));
        assert!(!valid_name(&"x".repeat(MAX_NAME_BYTES + 1)));
    }

    #[test]
    fn valid_name_rejects_newline_and_nul_bytes() {
        assert!(!valid_name("bad\nname"));
        assert!(!valid_name("bad\0name"));
    }

    #[test]
    fn encode_open_returns_none_for_invalid_name() {
        assert!(encode_open(3, "").is_none());
        assert!(encode_open(3, &"x".repeat(MAX_NAME_BYTES + 1)).is_none());
        assert!(encode_open(3, "bad\nname").is_none());
        assert!(encode_open(3, "bad\0name").is_none());
    }

    #[test]
    fn encode_delete_returns_none_for_invalid_name() {
        assert!(encode_delete("").is_none());
        assert!(encode_delete(&"x".repeat(MAX_NAME_BYTES + 1)).is_none());
        assert!(encode_delete("bad\nname").is_none());
        assert!(encode_delete("bad\0name").is_none());
    }

    #[test]
    fn ack_reply_decodes_when_tail_zero() {
        let frame = [0u8; 60];
        assert_eq!(decode_reply(&frame, Expect::Ack), Ok(Reply::Ack));
    }

    #[test]
    fn ack_reply_rejects_nonzero_tail() {
        let mut frame = [0u8; 60];
        frame[59] = 1;
        assert_eq!(decode_reply(&frame, Expect::Ack), Err(()));
    }

    #[test]
    fn opened_reply_frame_decodes_handle_at_exact_offsets() {
        let mut frame = [0u8; 60];
        frame[0] = STATUS_OK;
        frame[1] = 0xEF;
        frame[2] = 0xBE;
        frame[3] = 0xAD;
        frame[4] = 0xDE;
        assert_eq!(
            decode_reply(&frame, Expect::Opened),
            Ok(Reply::Opened {
                handle: 0xDEAD_BEEF
            })
        );
    }

    #[test]
    fn opened_reply_rejects_zero_handle() {
        let mut frame = [0u8; 60];
        frame[0] = STATUS_OK;
        assert_eq!(decode_reply(&frame, Expect::Opened), Err(()));
    }

    #[test]
    fn read_data_reply_frame_decodes_token_and_count_at_exact_offsets() {
        let mut frame = [0u8; 60];
        frame[0] = STATUS_OK;
        frame[1] = 0x78;
        frame[2] = 0x56;
        frame[3] = 0x34;
        frame[4] = 0x12;
        frame[5] = 0x00;
        frame[6] = 0x08;
        assert_eq!(
            decode_reply(&frame, Expect::ReadData),
            Ok(Reply::ReadData {
                token: 0x1234_5678,
                count: 0x0800
            })
        );
    }

    #[test]
    fn read_data_reply_rejects_zero_count() {
        let mut frame = [0u8; 60];
        frame[0] = STATUS_OK;
        frame[1] = 0xEF;
        frame[2] = 0xBE;
        frame[3] = 0xAD;
        frame[4] = 0xDE;
        assert_eq!(decode_reply(&frame, Expect::ReadData), Err(()));
    }

    #[test]
    fn written_reply_frame_decodes_bytes_at_exact_offsets() {
        let mut frame = [0u8; 60];
        frame[0] = STATUS_OK;
        frame[1] = 40;
        assert_eq!(
            decode_reply(&frame, Expect::Written),
            Ok(Reply::Written { bytes: 40 })
        );
    }

    #[test]
    fn written_reply_accepts_zero_bytes() {
        let frame = [0u8; 60];
        assert_eq!(
            decode_reply(&frame, Expect::Written),
            Ok(Reply::Written { bytes: 0 })
        );
    }

    #[test]
    fn listed_reply_frame_decodes_token_and_total_at_exact_offsets() {
        let mut frame = [0u8; 60];
        frame[0] = STATUS_OK;
        frame[1] = 0xEF;
        frame[2] = 0xBE;
        frame[3] = 0xAD;
        frame[4] = 0xDE;
        frame[5] = 0x00;
        frame[6] = 0x20;
        assert_eq!(
            decode_reply(&frame, Expect::Listed),
            Ok(Reply::Listed {
                token: 0xDEAD_BEEF,
                total: 0x2000
            })
        );
    }

    #[test]
    fn listed_reply_rejects_zero_total() {
        let mut frame = [0u8; 60];
        frame[0] = STATUS_OK;
        frame[1] = 0xEF;
        frame[2] = 0xBE;
        frame[3] = 0xAD;
        frame[4] = 0xDE;
        assert_eq!(decode_reply(&frame, Expect::Listed), Err(()));
    }

    #[test]
    fn read_data_reply_with_zero_token_decodes_ok() {
        let mut frame = [0u8; 60];
        frame[0] = STATUS_OK;
        frame[5] = 0x01;
        assert_eq!(
            decode_reply(&frame, Expect::ReadData),
            Ok(Reply::ReadData { token: 0, count: 1 })
        );
    }

    #[test]
    fn listed_reply_with_zero_token_decodes_ok() {
        let mut frame = [0u8; 60];
        frame[0] = STATUS_OK;
        frame[5] = 0x01;
        assert_eq!(
            decode_reply(&frame, Expect::Listed),
            Ok(Reply::Listed { token: 0, total: 1 })
        );
    }

    #[test]
    fn eof_status_decodes_under_every_expect() {
        let mut frame = [0u8; 60];
        frame[0] = STATUS_EOF;
        for expect in ALL_EXPECTS {
            assert_eq!(decode_reply(&frame, expect), Ok(Reply::Eof));
        }
    }

    #[test]
    fn eof_status_ignores_nonzero_tail_under_every_expect() {
        let mut frame = [0xFFu8; 60];
        frame[0] = STATUS_EOF;
        for expect in ALL_EXPECTS {
            assert_eq!(decode_reply(&frame, expect), Ok(Reply::Eof));
        }
    }

    #[test]
    fn err_status_carries_error_variant_under_every_expect() {
        let mut frame = [0u8; 60];
        frame[0] = STATUS_ERR;
        frame[1] = 4;
        for expect in ALL_EXPECTS {
            assert_eq!(
                decode_reply(&frame, expect),
                Ok(Reply::Err(VfsError::BadHandle))
            );
        }
    }

    #[test]
    fn err_status_ignores_tail_under_every_expect() {
        let mut frame = [0xFFu8; 60];
        frame[0] = STATUS_ERR;
        frame[1] = 2;
        for expect in ALL_EXPECTS {
            assert_eq!(
                decode_reply(&frame, expect),
                Ok(Reply::Err(VfsError::AlreadyExists))
            );
        }
    }

    #[test]
    fn unknown_status_byte_rejects_under_every_expect() {
        for status in [3u8, 128, 255] {
            let mut frame = [0u8; 60];
            frame[0] = status;
            for expect in ALL_EXPECTS {
                assert_eq!(decode_reply(&frame, expect), Err(()));
            }
        }
    }

    #[test]
    fn ack_shaped_frame_rejects_opened_read_data_and_listed_expects() {
        let frame = [0u8; 60];
        assert_eq!(decode_reply(&frame, Expect::Opened), Err(()));
        assert_eq!(decode_reply(&frame, Expect::ReadData), Err(()));
        assert_eq!(decode_reply(&frame, Expect::Listed), Err(()));
    }

    #[test]
    fn data_bearing_ok_frame_rejects_ack_expect() {
        let mut frame = [0u8; 60];
        frame[0] = STATUS_OK;
        frame[1] = 0x78;
        frame[5] = 0x01;
        assert_eq!(decode_reply(&frame, Expect::Ack), Err(()));
    }

    #[test]
    fn every_error_code_roundtrips_on_the_wire() {
        for (byte, error) in WIRE_ERRORS {
            let mut frame = [0u8; 60];
            frame[0] = STATUS_ERR;
            frame[1] = byte;
            assert_eq!(decode_reply(&frame, Expect::Ack), Ok(Reply::Err(error)));
            assert_eq!(VfsError::from_byte(byte), error);
        }
    }

    #[test]
    fn from_byte_is_total_and_maps_unknown_to_protocol() {
        for byte in 0u8..=255 {
            let expected = WIRE_ERRORS
                .iter()
                .find(|(code, _)| *code == byte)
                .map(|(_, error)| *error)
                .unwrap_or(VfsError::Protocol);
            assert_eq!(VfsError::from_byte(byte), expected);
        }
    }

    #[test]
    fn unknown_error_byte_maps_to_protocol_not_server_resource() {
        for byte in [0u8, 8, 255] {
            assert_eq!(VfsError::from_byte(byte), VfsError::Protocol);
            assert_ne!(VfsError::from_byte(byte), VfsError::ServerResource);
        }
    }
}
