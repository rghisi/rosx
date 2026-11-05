pub const FRAME_BYTES: usize = 60;

pub const MAX_BLOCKS_PER_REQUEST: usize = 128;
pub const DESCRIPTION_BYTES: usize = 47;

pub const PROBE: u8 = 0x20;
pub const READ: u8 = 0x21;
pub const WRITE: u8 = 0x22;
pub const FLUSH: u8 = 0x23;

pub const REPLY_ACK: u8 = 0x30;
pub const REPLY_ERROR: u8 = 0x31;
pub const REPLY_PROBE: u8 = 0x32;
pub const REPLY_DATA: u8 = 0x33;

pub fn is_block_request(opcode: u8) -> bool {
    (PROBE..=FLUSH).contains(&opcode)
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[repr(u8)]
pub enum BlockError {
    NotFound = 1,
    BadAddress = 2,
    Busy = 3,
    Device = 4,
    Unsupported = 5,
    Protocol = 255,
}

impl BlockError {
    pub fn to_byte(self) -> u8 {
        self as u8
    }

    pub fn from_byte(byte: u8) -> BlockError {
        match byte {
            1 => BlockError::NotFound,
            2 => BlockError::BadAddress,
            3 => BlockError::Busy,
            4 => BlockError::Device,
            5 => BlockError::Unsupported,
            _ => BlockError::Protocol,
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum Request {
    Probe { device: u32 },
    Read { device: u32, block: u64, count: u16 },
    Write { device: u32, block: u64, count: u16, token: u32 },
    Flush { device: u32 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reply<'a> {
    Ack,
    Err(BlockError),
    Probe {
        block_count: u64,
        block_size: u32,
        description: &'a str,
    },
    Data {
        token: u32,
    },
}

fn valid_count(count: u16) -> bool {
    count >= 1 && count as usize <= MAX_BLOCKS_PER_REQUEST
}

fn valid_block_size(block_size: u32) -> bool {
    block_size.is_power_of_two()
}

impl Request {
    pub fn encode(&self) -> Result<[u8; FRAME_BYTES], BlockError> {
        let mut frame = [0u8; FRAME_BYTES];
        match self {
            Request::Probe { device } => {
                frame[0] = PROBE;
                frame[1..5].copy_from_slice(&device.to_le_bytes());
            }
            Request::Read { device, block, count } => {
                if !valid_count(*count) {
                    return Err(BlockError::Protocol);
                }
                frame[0] = READ;
                frame[1..5].copy_from_slice(&device.to_le_bytes());
                frame[5..13].copy_from_slice(&block.to_le_bytes());
                frame[13..15].copy_from_slice(&count.to_le_bytes());
            }
            Request::Write {
                device,
                block,
                count,
                token,
            } => {
                if !valid_count(*count) {
                    return Err(BlockError::Protocol);
                }
                frame[0] = WRITE;
                frame[1..5].copy_from_slice(&device.to_le_bytes());
                frame[5..13].copy_from_slice(&block.to_le_bytes());
                frame[13..15].copy_from_slice(&count.to_le_bytes());
                frame[15..19].copy_from_slice(&token.to_le_bytes());
            }
            Request::Flush { device } => {
                frame[0] = FLUSH;
                frame[1..5].copy_from_slice(&device.to_le_bytes());
            }
        }
        Ok(frame)
    }

    pub fn decode(frame: &[u8; FRAME_BYTES]) -> Result<Request, BlockError> {
        match frame[0] {
            PROBE => Ok(Request::Probe {
                device: read_u32(frame, 1),
            }),
            READ => {
                let (block, count) = decode_geometry(frame)?;
                Ok(Request::Read {
                    device: read_u32(frame, 1),
                    block,
                    count,
                })
            }
            WRITE => {
                let (block, count) = decode_geometry(frame)?;
                Ok(Request::Write {
                    device: read_u32(frame, 1),
                    block,
                    count,
                    token: read_u32(frame, 15),
                })
            }
            FLUSH => Ok(Request::Flush {
                device: read_u32(frame, 1),
            }),
            _ => Err(BlockError::Protocol),
        }
    }
}

fn decode_geometry(frame: &[u8; FRAME_BYTES]) -> Result<(u64, u16), BlockError> {
    let count = read_u16(frame, 13);
    if !valid_count(count) {
        return Err(BlockError::Protocol);
    }
    Ok((read_u64(frame, 5), count))
}

impl Reply<'_> {
    pub fn encode(&self) -> Result<[u8; FRAME_BYTES], BlockError> {
        let mut frame = [0u8; FRAME_BYTES];
        match self {
            Reply::Ack => frame[0] = REPLY_ACK,
            Reply::Err(error) => {
                frame[0] = REPLY_ERROR;
                frame[1] = error.to_byte();
            }
            Reply::Probe {
                block_count,
                block_size,
                description,
            } => {
                if !valid_block_size(*block_size) {
                    return Err(BlockError::Protocol);
                }
                if description.len() > DESCRIPTION_BYTES {
                    return Err(BlockError::Protocol);
                }
                frame[0] = REPLY_PROBE;
                frame[1..9].copy_from_slice(&block_count.to_le_bytes());
                frame[9..13].copy_from_slice(&block_size.to_le_bytes());
                put_description(&mut frame, description.as_bytes());
            }
            Reply::Data { token } => {
                frame[0] = REPLY_DATA;
                frame[1..5].copy_from_slice(&token.to_le_bytes());
            }
        }
        Ok(frame)
    }

    pub fn decode(frame: &[u8; FRAME_BYTES]) -> Result<Reply<'_>, BlockError> {
        match frame[0] {
            REPLY_ACK => {
                zero_tail(frame, 1)?;
                Ok(Reply::Ack)
            }
            REPLY_ERROR => {
                zero_tail(frame, 2)?;
                Ok(Reply::Err(BlockError::from_byte(frame[1])))
            }
            REPLY_PROBE => {
                let block_size = read_u32(frame, 9);
                if !valid_block_size(block_size) {
                    return Err(BlockError::Protocol);
                }
                Ok(Reply::Probe {
                    block_count: read_u64(frame, 1),
                    block_size,
                    description: description_str(frame)?,
                })
            }
            REPLY_DATA => {
                zero_tail(frame, 5)?;
                Ok(Reply::Data {
                    token: read_u32(frame, 1),
                })
            }
            _ => Err(BlockError::Protocol),
        }
    }
}

fn put_description(frame: &mut [u8; FRAME_BYTES], bytes: &[u8]) {
    for offset in 0..DESCRIPTION_BYTES {
        let byte = match bytes.get(offset) {
            Some(byte) if (0x20..=0x7E).contains(byte) => *byte,
            _ => b' ',
        };
        frame[13 + offset] = byte;
    }
}

fn description_str(frame: &[u8; FRAME_BYTES]) -> Result<&str, BlockError> {
    let field = &frame[13..13 + DESCRIPTION_BYTES];
    let text = core::str::from_utf8(field).map_err(|_| BlockError::Protocol)?;
    Ok(text.trim_end_matches(' '))
}

fn zero_tail(frame: &[u8; FRAME_BYTES], from: usize) -> Result<(), BlockError> {
    if frame[from..].iter().any(|byte| *byte != 0) {
        Err(BlockError::Protocol)
    } else {
        Ok(())
    }
}

fn read_u16(frame: &[u8; FRAME_BYTES], at: usize) -> u16 {
    u16::from_le_bytes(frame[at..at + 2].try_into().unwrap())
}

fn read_u32(frame: &[u8; FRAME_BYTES], at: usize) -> u32 {
    u32::from_le_bytes(frame[at..at + 4].try_into().unwrap())
}

fn read_u64(frame: &[u8; FRAME_BYTES], at: usize) -> u64 {
    u64::from_le_bytes(frame[at..at + 8].try_into().unwrap())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn description_of_len(length: usize) -> alloc::string::String {
        (0..length).map(|_| 'k').collect()
    }

    #[test]
    fn opcode_band_is_pinned_and_contiguous() {
        assert_eq!(PROBE, 0x20);
        assert_eq!(READ, 0x21);
        assert_eq!(WRITE, 0x22);
        assert_eq!(FLUSH, 0x23);
        assert_eq!(REPLY_ACK, 0x30);
        assert_eq!(REPLY_ERROR, 0x31);
        assert_eq!(REPLY_PROBE, 0x32);
        assert_eq!(REPLY_DATA, 0x33);
    }

    #[test]
    fn band_membership_covers_exactly_the_four_opcodes() {
        for opcode in [PROBE, READ, WRITE, FLUSH] {
            assert!(is_block_request(opcode), "opcode {opcode:#04x} rejected");
        }
        for opcode in [0x00u8, 0x04, 0x13, 0x1F, 0x24, 0x2F, 0x30, 0xFF] {
            assert!(!is_block_request(opcode), "opcode {opcode:#04x} accepted");
        }
    }

    #[test]
    fn probe_frame_pins_device_offset() {
        let frame = Request::Probe { device: 0x0A0B_0C0D }.encode().unwrap();
        assert_eq!(frame[0], PROBE);
        assert_eq!(&frame[1..5], &[0x0D, 0x0C, 0x0B, 0x0A]);
        assert!(frame[5..].iter().all(|byte| *byte == 0));
    }

    #[test]
    fn read_frame_pins_little_endian_offsets() {
        let frame = Request::Read {
            device: 1,
            block: 0x0807_0605_0403_0201,
            count: 8,
        }
        .encode()
        .unwrap();
        assert_eq!(frame[0], READ);
        assert_eq!(&frame[1..5], &[0x01, 0x00, 0x00, 0x00]);
        assert_eq!(
            &frame[5..13],
            &[0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08]
        );
        assert_eq!(&frame[13..15], &[0x08, 0x00]);
        assert!(frame[15..].iter().all(|byte| *byte == 0));
    }

    #[test]
    fn write_frame_pins_little_endian_offsets() {
        let frame = Request::Write {
            device: 0xFFFF_FFFF,
            block: 4096,
            count: 128,
            token: 0x00AB_CDEF,
        }
        .encode()
        .unwrap();
        assert_eq!(frame[0], WRITE);
        assert_eq!(&frame[1..5], &[0xFF, 0xFF, 0xFF, 0xFF]);
        assert_eq!(
            &frame[5..13],
            &[0x00, 0x10, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]
        );
        assert_eq!(&frame[13..15], &[0x80, 0x00]);
        assert_eq!(&frame[15..19], &[0xEF, 0xCD, 0xAB, 0x00]);
        assert!(frame[19..].iter().all(|byte| *byte == 0));
    }

    #[test]
    fn flush_frame_pins_device_offset() {
        let frame = Request::Flush { device: 3 }.encode().unwrap();
        assert_eq!(frame[0], FLUSH);
        assert_eq!(&frame[1..5], &[0x03, 0x00, 0x00, 0x00]);
        assert!(frame[5..].iter().all(|byte| *byte == 0));
    }

    #[test]
    fn request_roundtrips_all_variants() {
        let requests = [
            Request::Probe { device: 0 },
            Request::Probe { device: u32::MAX },
            Request::Read {
                device: 1,
                block: u64::MAX,
                count: 1,
            },
            Request::Read {
                device: 0,
                block: 0,
                count: MAX_BLOCKS_PER_REQUEST as u16,
            },
            Request::Write {
                device: 7,
                block: 9,
                count: 33,
                token: 0x7FFF_FFFF,
            },
            Request::Flush { device: u32::MAX },
        ];
        for request in requests {
            assert_eq!(Request::decode(&request.encode().unwrap()), Ok(request));
        }
    }

    #[test]
    fn decode_request_rejects_opcodes_outside_the_band() {
        for op in [0u8, 0x01, 0x04, 0x13, 0x1F, 0x24, 0x2F, 0x30, 0xFF] {
            let mut frame = Request::Flush { device: 0 }.encode().unwrap();
            frame[0] = op;
            assert_eq!(
                Request::decode(&frame),
                Err(BlockError::Protocol),
                "op byte {op:#04x} accepted"
            );
        }
    }

    #[test]
    fn request_bounds_reject_count_zero_and_above_maximum() {
        for count in [0u16, 129, u16::MAX] {
            let mut frame = Request::Read {
                device: 0,
                block: 0,
                count: 1,
            }
            .encode()
            .unwrap();
            frame[13..15].copy_from_slice(&count.to_le_bytes());
            assert_eq!(
                Request::decode(&frame),
                Err(BlockError::Protocol),
                "count {count} accepted"
            );
            assert_eq!(
                Request::Read {
                    device: 0,
                    block: 0,
                    count,
                }
                .encode(),
                Err(BlockError::Protocol)
            );
        }
    }

    #[test]
    fn write_decode_enforces_the_count_bounds() {
        for count in [0u16, 129] {
            let mut frame = Request::Write {
                device: 0,
                block: 0,
                count: 1,
                token: 5,
            }
            .encode()
            .unwrap();
            frame[13..15].copy_from_slice(&count.to_le_bytes());
            assert_eq!(Request::decode(&frame), Err(BlockError::Protocol));
        }
    }

    #[test]
    fn max_blocks_pins_the_transfer_cap_at_512_byte_width() {
        assert_eq!(MAX_BLOCKS_PER_REQUEST * 512, 64 * 1024);
    }

    #[test]
    fn ack_reply_pins_tag_and_zero_tail() {
        let frame = Reply::Ack.encode().unwrap();
        assert_eq!(frame[0], REPLY_ACK);
        assert!(frame[1..].iter().all(|byte| *byte == 0));
    }

    #[test]
    fn error_reply_pins_tag_and_error_byte() {
        let frame = Reply::Err(BlockError::Device).encode().unwrap();
        assert_eq!(frame[0], REPLY_ERROR);
        assert_eq!(frame[1], 4);
        assert!(frame[2..].iter().all(|byte| *byte == 0));
    }

    #[test]
    fn probe_reply_pins_field_offsets() {
        let frame = Reply::Probe {
            block_count: 0x0000_0100_0000_0001,
            block_size: 4096,
            description: "QEMU HARDDISK",
        }
        .encode()
        .unwrap();
        assert_eq!(frame[0], REPLY_PROBE);
        assert_eq!(
            &frame[1..9],
            &[0x01, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00]
        );
        assert_eq!(&frame[9..13], &[0x00, 0x10, 0x00, 0x00]);
        assert_eq!(&frame[13..26], b"QEMU HARDDISK");
        assert!(frame[26..].iter().all(|byte| *byte == b' '));
    }

    #[test]
    fn probe_reply_accepts_the_full_description_width() {
        let description = description_of_len(DESCRIPTION_BYTES);
        let frame = Reply::Probe {
            block_count: 1,
            block_size: 512,
            description: &description,
        }
        .encode()
        .unwrap();
        assert_eq!(
            Reply::decode(&frame),
            Ok(Reply::Probe {
                block_count: 1,
                block_size: 512,
                description: description.as_str(),
            })
        );
    }

    #[test]
    fn probe_reply_rejects_an_overlong_description() {
        assert_eq!(
            Reply::Probe {
                block_count: 1,
                block_size: 512,
                description: &description_of_len(DESCRIPTION_BYTES + 1),
            }
            .encode(),
            Err(BlockError::Protocol)
        );
    }

    #[test]
    fn probe_reply_normalizes_non_printable_description_bytes() {
        let frame = Reply::Probe {
            block_count: 1,
            block_size: 512,
            description: "ok\x01\x7f!",
        }
        .encode()
        .unwrap();
        assert_eq!(&frame[13..17], b"ok  ");
        assert_eq!(&frame[17..19], b"! ");
    }

    #[test]
    fn probe_reply_decode_rejects_invalid_utf8_description() {
        let mut frame = Reply::Probe {
            block_count: 1,
            block_size: 512,
            description: "ok",
        }
        .encode()
        .unwrap();
        frame[15] = 0xC3;
        frame[16] = 0x28;
        assert_eq!(Reply::decode(&frame), Err(BlockError::Protocol));
    }

    #[test]
    fn probe_reply_rejects_block_size_zero_and_non_power_of_two() {
        for block_size in [0u32, 1000, 3, u32::MAX] {
            assert_eq!(
                Reply::Probe {
                    block_count: 1,
                    block_size,
                    description: "d",
                }
                .encode(),
                Err(BlockError::Protocol),
                "block size {block_size} accepted on encode"
            );
            let mut frame = Reply::Probe {
                block_count: 1,
                block_size: 4096,
                description: "d",
            }
            .encode()
            .unwrap();
            frame[9..13].copy_from_slice(&block_size.to_le_bytes());
            assert_eq!(
                Reply::decode(&frame),
                Err(BlockError::Protocol),
                "block size {block_size} accepted on decode"
            );
        }
    }

    #[test]
    fn probe_reply_decode_accepts_zero_block_count() {
        let frame = Reply::Probe {
            block_count: 0,
            block_size: 512,
            description: "",
        }
        .encode()
        .unwrap();
        assert_eq!(
            Reply::decode(&frame),
            Ok(Reply::Probe {
                block_count: 0,
                block_size: 512,
                description: "",
            })
        );
    }

    #[test]
    fn data_reply_pins_token_offset() {
        let frame = Reply::Data { token: 0x0102_0304 }.encode().unwrap();
        assert_eq!(frame[0], REPLY_DATA);
        assert_eq!(&frame[1..5], &[0x04, 0x03, 0x02, 0x01]);
        assert!(frame[5..].iter().all(|byte| *byte == 0));
    }

    #[test]
    fn reply_all_variants_roundtrip() {
        let replies = [
            Reply::Ack,
            Reply::Err(BlockError::NotFound),
            Reply::Err(BlockError::Unsupported),
            Reply::Probe {
                block_count: 12800,
                block_size: 4096,
                description: "QEMU HARDDISK 50MB",
            },
            Reply::Probe {
                block_count: u64::MAX,
                block_size: 512,
                description: &description_of_len(DESCRIPTION_BYTES),
            },
            Reply::Data { token: 9 },
            Reply::Data { token: 0 },
        ];
        for reply in replies {
            assert_eq!(Reply::decode(&reply.encode().unwrap()), Ok(reply));
        }
    }

    #[test]
    fn reply_decode_rejects_unknown_tag() {
        for tag in [0x00u8, 0x2F, 0x34, 0xFF] {
            let mut frame = [0u8; FRAME_BYTES];
            frame[0] = tag;
            assert_eq!(Reply::decode(&frame), Err(BlockError::Protocol), "tag {tag:#04x} accepted");
        }
    }

    #[test]
    fn reply_decode_rejects_nonzero_ack_tail() {
        let mut frame = Reply::Ack.encode().unwrap();
        frame[30] = 0x40;
        assert_eq!(Reply::decode(&frame), Err(BlockError::Protocol));
    }

    #[test]
    fn reply_decode_rejects_nonzero_error_tail() {
        let mut frame = Reply::Err(BlockError::Busy).encode().unwrap();
        frame[2] = 1;
        assert_eq!(Reply::decode(&frame), Err(BlockError::Protocol));
    }

    #[test]
    fn reply_decode_rejects_nonzero_data_tail() {
        let mut frame = Reply::Data { token: 1 }.encode().unwrap();
        frame[5] = 1;
        assert_eq!(Reply::decode(&frame), Err(BlockError::Protocol));
    }

    #[test]
    fn wire_error_codes_roundtrip() {
        let codes = [
            BlockError::NotFound,
            BlockError::BadAddress,
            BlockError::Busy,
            BlockError::Device,
            BlockError::Unsupported,
        ];
        for code in codes {
            assert_eq!(BlockError::from_byte(code.to_byte()), code);
        }
        assert_eq!(BlockError::NotFound.to_byte(), 1);
        assert_eq!(BlockError::Unsupported.to_byte(), 5);
        assert_eq!(BlockError::Protocol.to_byte(), 255);
    }

    #[test]
    fn unknown_wire_error_bytes_map_to_protocol() {
        for byte in [0u8, 6, 128, 254] {
            assert_eq!(BlockError::from_byte(byte), BlockError::Protocol);
        }
    }

    fn seed_frames() -> Vec<[u8; FRAME_BYTES]> {
        let requests = [
            Request::Probe { device: 0 },
            Request::Read {
                device: 1,
                block: 7105,
                count: 1,
            },
            Request::Write {
                device: 0,
                block: 0,
                count: 128,
                token: 0x00AB_CDEF,
            },
            Request::Flush { device: 1 },
        ];
        let replies = [
            Reply::Ack,
            Reply::Err(BlockError::Device),
            Reply::Probe {
                block_count: 12800,
                block_size: 4096,
                description: "QEMU HARDDISK",
            },
            Reply::Data { token: 9 },
        ];
        let mut frames = Vec::new();
        for request in requests {
            frames.push(request.encode().expect("seed request encodes"));
        }
        for reply in replies {
            frames.push(reply.encode().expect("seed reply encodes"));
        }
        frames
    }

    #[test]
    fn decoding_flips_never_panics() {
        for seed in seed_frames() {
            for byte in 0..FRAME_BYTES {
                for bit in 0..8 {
                    let mut mutated = seed;
                    mutated[byte] ^= 1 << bit;
                    let _ = Request::decode(&mutated);
                    let _ = Reply::decode(&mutated);
                }
            }
        }
    }
}
