pub const SERVICE_NAME: &str = "ATA";
pub const FRAME_BYTES: usize = 60;
pub const SECTOR_BYTES: usize = 512;
pub const MAX_SECTORS_PER_REQUEST: usize = 128;

pub const LBA28_MAX: u32 = 0x0F00_0000 - 1;

pub const IDENTIFY: u8 = 0x01;
pub const READ: u8 = 0x02;
pub const WRITE: u8 = 0x03;
pub const FLUSH: u8 = 0x04;

pub const REPLY_ACK: u8 = 0x10;
pub const REPLY_ERROR: u8 = 0x11;
pub const REPLY_IDENT: u8 = 0x12;
pub const REPLY_DATA: u8 = 0x13;

pub const MODEL_BYTES: usize = 40;
pub const FIRMWARE_BYTES: usize = 8;

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[repr(u8)]
pub enum AtaError {
    NotFound = 1,
    BadAddress = 2,
    Busy = 3,
    Device = 4,
    Denied = 5,
    Protocol = 255,
}

impl AtaError {
    pub fn to_byte(self) -> u8 {
        self as u8
    }

    pub fn from_byte(byte: u8) -> AtaError {
        match byte {
            1 => AtaError::NotFound,
            2 => AtaError::BadAddress,
            3 => AtaError::Busy,
            4 => AtaError::Device,
            5 => AtaError::Denied,
            _ => AtaError::Protocol,
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum Request {
    Identify { drive: u8 },
    Read { drive: u8, lba: u32, count: u8 },
    Write { drive: u8, lba: u32, count: u8, token: u32 },
    Flush { drive: u8 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reply<'a> {
    Ack,
    Err {
        error: AtaError,
        error_register: u8,
        status: u8,
    },
    Ident {
        sectors: u64,
        firmware_rev: &'a str,
        model: &'a str,
    },
    Data {
        token: u32,
        sectors: u16,
    },
}

fn valid_drive(drive: u8) -> bool {
    drive <= 1
}

fn valid_lba(lba: u32) -> bool {
    lba <= LBA28_MAX
}

fn valid_count(count: u8) -> bool {
    count >= 1 && count as usize <= MAX_SECTORS_PER_REQUEST
}

impl Request {
    pub fn encode(&self) -> Result<[u8; FRAME_BYTES], AtaError> {
        let mut frame = [0u8; FRAME_BYTES];
        match self {
            Request::Identify { drive } => {
                if !valid_drive(*drive) {
                    return Err(AtaError::Protocol);
                }
                frame[0] = IDENTIFY;
                frame[1] = *drive;
            }
            Request::Read { drive, lba, count } => {
                if !valid_drive(*drive) || !valid_lba(*lba) || !valid_count(*count) {
                    return Err(AtaError::Protocol);
                }
                frame[0] = READ;
                frame[1] = *drive;
                frame[2..6].copy_from_slice(&lba.to_le_bytes());
                frame[6] = *count;
            }
            Request::Write {
                drive,
                lba,
                count,
                token,
            } => {
                if !valid_drive(*drive) || !valid_lba(*lba) || !valid_count(*count) {
                    return Err(AtaError::Protocol);
                }
                frame[0] = WRITE;
                frame[1] = *drive;
                frame[2..6].copy_from_slice(&lba.to_le_bytes());
                frame[6] = *count;
                frame[7..11].copy_from_slice(&token.to_le_bytes());
            }
            Request::Flush { drive } => {
                if !valid_drive(*drive) {
                    return Err(AtaError::Protocol);
                }
                frame[0] = FLUSH;
                frame[1] = *drive;
            }
        }
        Ok(frame)
    }

    pub fn decode(frame: &[u8; FRAME_BYTES]) -> Result<Request, AtaError> {
        match frame[0] {
            IDENTIFY => {
                let drive = frame[1];
                if !valid_drive(drive) {
                    return Err(AtaError::Protocol);
                }
                Ok(Request::Identify { drive })
            }
            READ => {
                let (drive, lba, count) = decode_geometry(frame)?;
                Ok(Request::Read { drive, lba, count })
            }
            WRITE => {
                let (drive, lba, count) = decode_geometry(frame)?;
                Ok(Request::Write {
                    drive,
                    lba,
                    count,
                    token: read_u32(frame, 7),
                })
            }
            FLUSH => {
                let drive = frame[1];
                if !valid_drive(drive) {
                    return Err(AtaError::Protocol);
                }
                Ok(Request::Flush { drive })
            }
            _ => Err(AtaError::Protocol),
        }
    }
}

fn decode_geometry(frame: &[u8; FRAME_BYTES]) -> Result<(u8, u32, u8), AtaError> {
    let drive = frame[1];
    let lba = read_u32(frame, 2);
    let count = frame[6];
    if !valid_drive(drive) || !valid_lba(lba) || !valid_count(count) {
        return Err(AtaError::Protocol);
    }
    Ok((drive, lba, count))
}

impl Reply<'_> {
    pub fn encode(&self) -> Result<[u8; FRAME_BYTES], AtaError> {
        let mut frame = [0u8; FRAME_BYTES];
        match self {
            Reply::Ack => frame[0] = REPLY_ACK,
            Reply::Err {
                error,
                error_register,
                status,
            } => {
                frame[0] = REPLY_ERROR;
                frame[1] = error.to_byte();
                frame[2] = *error_register;
                frame[3] = *status;
            }
            Reply::Ident {
                sectors,
                firmware_rev,
                model,
            } => {
                if firmware_rev.len() > FIRMWARE_BYTES || model.len() > MODEL_BYTES {
                    return Err(AtaError::Protocol);
                }
                frame[0] = REPLY_IDENT;
                frame[1..9].copy_from_slice(&sectors.to_le_bytes());
                put_identity(&mut frame, 9, FIRMWARE_BYTES, firmware_rev.as_bytes());
                put_identity(&mut frame, 9 + FIRMWARE_BYTES, MODEL_BYTES, model.as_bytes());
            }
            Reply::Data { token, sectors } => {
                if *sectors == 0 || *sectors as usize > MAX_SECTORS_PER_REQUEST {
                    return Err(AtaError::Protocol);
                }
                frame[0] = REPLY_DATA;
                frame[1..5].copy_from_slice(&token.to_le_bytes());
                frame[5..7].copy_from_slice(&sectors.to_le_bytes());
            }
        }
        Ok(frame)
    }

    pub fn decode(frame: &[u8; FRAME_BYTES]) -> Result<Reply<'_>, AtaError> {
        match frame[0] {
            REPLY_ACK => {
                zero_tail(frame, 1)?;
                Ok(Reply::Ack)
            }
            REPLY_ERROR => {
                zero_tail(frame, 4)?;
                Ok(Reply::Err {
                    error: AtaError::from_byte(frame[1]),
                    error_register: frame[2],
                    status: frame[3],
                })
            }
            REPLY_IDENT => {
                zero_tail(frame, 9 + FIRMWARE_BYTES + MODEL_BYTES)?;
                let firmware_rev = identity_str(frame, 9, FIRMWARE_BYTES)?;
                let model = identity_str(frame, 9 + FIRMWARE_BYTES, MODEL_BYTES)?;
                Ok(Reply::Ident {
                    sectors: read_u64(frame, 1),
                    firmware_rev,
                    model,
                })
            }
            REPLY_DATA => {
                zero_tail(frame, 7)?;
                let sectors = u16::from_le_bytes(frame[5..7].try_into().unwrap());
                if sectors == 0 || sectors as usize > MAX_SECTORS_PER_REQUEST {
                    return Err(AtaError::Protocol);
                }
                Ok(Reply::Data {
                    token: read_u32(frame, 1),
                    sectors,
                })
            }
            _ => Err(AtaError::Protocol),
        }
    }
}

fn put_identity(frame: &mut [u8; FRAME_BYTES], at: usize, width: usize, bytes: &[u8]) {
    for offset in 0..width {
        let byte = match bytes.get(offset) {
            Some(byte) if (0x20..=0x7E).contains(byte) => *byte,
            _ => b' ',
        };
        frame[at + offset] = byte;
    }
}

fn identity_str(frame: &[u8; FRAME_BYTES], at: usize, width: usize) -> Result<&str, AtaError> {
    let field = &frame[at..at + width];
    let text = core::str::from_utf8(field).map_err(|_| AtaError::Protocol)?;
    Ok(text.trim_end_matches(' '))
}

fn zero_tail(frame: &[u8; FRAME_BYTES], from: usize) -> Result<(), AtaError> {
    if frame[from..].iter().any(|byte| *byte != 0) {
        Err(AtaError::Protocol)
    } else {
        Ok(())
    }
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

    fn model_of_len(length: usize) -> alloc::string::String {
        (0..length).map(|_| 'k').collect()
    }

    #[test]
    fn identify_frame_pins_offsets() {
        let frame = Request::Identify { drive: 1 }.encode().unwrap();
        assert_eq!(frame[0], IDENTIFY);
        assert_eq!(frame[1], 1);
        assert!(frame[2..].iter().all(|byte| *byte == 0));
    }

    #[test]
    fn read_frame_pins_little_endian_offsets() {
        let frame = Request::Read {
            drive: 0,
            lba: 0x0ABC_DEF1,
            count: 128,
        }
        .encode()
        .unwrap();
        assert_eq!(frame[0], READ);
        assert_eq!(frame[1], 0);
        assert_eq!(&frame[2..6], &[0xF1, 0xDE, 0xBC, 0x0A]);
        assert_eq!(frame[6], 128);
        assert!(frame[7..].iter().all(|byte| *byte == 0));
    }

    #[test]
    fn write_frame_pins_little_endian_offsets() {
        let frame = Request::Write {
            drive: 1,
            lba: 0x0000_0100,
            count: 8,
            token: 0x0A0B_0C0D,
        }
        .encode()
        .unwrap();
        assert_eq!(frame[0], WRITE);
        assert_eq!(frame[1], 1);
        assert_eq!(&frame[2..6], &[0x00, 0x01, 0x00, 0x00]);
        assert_eq!(frame[6], 8);
        assert_eq!(&frame[7..11], &[0x0D, 0x0C, 0x0B, 0x0A]);
        assert!(frame[11..].iter().all(|byte| *byte == 0));
    }

    #[test]
    fn flush_frame_uses_op_and_drive_bytes_only() {
        let frame = Request::Flush { drive: 0 }.encode().unwrap();
        assert_eq!(frame[0], FLUSH);
        assert_eq!(frame[1], 0);
        assert!(frame[2..].iter().all(|byte| *byte == 0));
    }

    #[test]
    fn request_roundtrips_all_variants() {
        let requests = [
            Request::Identify { drive: 0 },
            Request::Identify { drive: 1 },
            Request::Read {
                drive: 1,
                lba: LBA28_MAX,
                count: 1,
            },
            Request::Read {
                drive: 0,
                lba: 0,
                count: MAX_SECTORS_PER_REQUEST as u8,
            },
            Request::Write {
                drive: 1,
                lba: 4096,
                count: 33,
                token: 0x7FFF_FFFF,
            },
            Request::Flush { drive: 1 },
        ];
        for request in requests {
            assert_eq!(Request::decode(&request.encode().unwrap()), Ok(request));
        }
    }

    #[test]
    fn decode_request_rejects_unknown_op_bytes() {
        for op in [0u8, 5, 0x0F, 0xFF] {
            let mut frame = Request::Flush { drive: 0 }.encode().unwrap();
            frame[0] = op;
            assert_eq!(
                Request::decode(&frame),
                Err(AtaError::Protocol),
                "op byte {op:#04x} accepted"
            );
        }
    }

    #[test]
    fn request_bounds_reject_drive_above_one() {
        let mut frame = Request::Identify { drive: 0 }.encode().unwrap();
        frame[1] = 2;
        assert_eq!(Request::decode(&frame), Err(AtaError::Protocol));
        assert_eq!(
            Request::Identify { drive: 2 }.encode(),
            Err(AtaError::Protocol)
        );
    }

    #[test]
    fn request_bounds_reject_lba_bits_28_through_31() {
        for lba in [0x1000_0000u32, 0x8000_0000, u32::MAX] {
            let mut frame = Request::Read {
                drive: 0,
                lba: 0,
                count: 1,
            }
            .encode()
            .unwrap();
            frame[2..6].copy_from_slice(&lba.to_le_bytes());
            assert_eq!(
                Request::decode(&frame),
                Err(AtaError::Protocol),
                "lba {lba:#010x} accepted"
            );
        }
        assert!(Request::Read {
            drive: 0,
            lba: LBA28_MAX,
            count: 1,
        }
        .encode()
        .is_ok());
    }

    #[test]
    fn request_bounds_reject_count_zero_and_above_maximum() {
        for count in [0u8, 129, 255] {
            let mut frame = Request::Read {
                drive: 0,
                lba: 0,
                count: 1,
            }
            .encode()
            .unwrap();
            frame[6] = count;
            assert_eq!(
                Request::decode(&frame),
                Err(AtaError::Protocol),
                "count {count} accepted"
            );
            assert_eq!(
                Request::Read {
                    drive: 0,
                    lba: 0,
                    count,
                }
                .encode(),
                Err(AtaError::Protocol)
            );
        }
    }

    #[test]
    fn sector_count_capacity_matches_the_data_object_cap() {
        assert_eq!(MAX_SECTORS_PER_REQUEST * SECTOR_BYTES, 64 * 1024);
    }

    #[test]
    fn tag_constants_are_pinned_and_disjoint_from_opcodes() {
        assert_eq!(IDENTIFY, 0x01);
        assert_eq!(READ, 0x02);
        assert_eq!(WRITE, 0x03);
        assert_eq!(FLUSH, 0x04);
        assert_eq!(REPLY_ACK, 0x10);
        assert_eq!(REPLY_ERROR, 0x11);
        assert_eq!(REPLY_IDENT, 0x12);
        assert_eq!(REPLY_DATA, 0x13);
        let opcodes = [IDENTIFY, READ, WRITE, FLUSH];
        let tags = [REPLY_ACK, REPLY_ERROR, REPLY_IDENT, REPLY_DATA];
        for tag in tags {
            assert!(!opcodes.contains(&tag), "tag {tag:#04x} collides with opcode");
        }
    }

    #[test]
    fn ack_reply_pins_tag_and_zero_tail() {
        let frame = Reply::Ack.encode().unwrap();
        assert_eq!(frame[0], REPLY_ACK);
        assert!(frame[1..].iter().all(|byte| *byte == 0));
    }

    #[test]
    fn error_reply_pins_snapshot_offsets() {
        let frame = Reply::Err {
            error: AtaError::Device,
            error_register: 0x41,
            status: 0xD0,
        }
        .encode()
        .unwrap();
        assert_eq!(frame[0], REPLY_ERROR);
        assert_eq!(frame[1], 4);
        assert_eq!(frame[2], 0x41);
        assert_eq!(frame[3], 0xD0);
        assert!(frame[4..].iter().all(|byte| *byte == 0));
    }

    #[test]
    fn ident_reply_pins_field_offsets() {
        let frame = Reply::Ident {
            sectors: 0x0000_0100_0000_0001,
            firmware_rev: "2.5+",
            model: "QEMU HARDDISK",
        }
        .encode()
        .unwrap();
        assert_eq!(frame[0], REPLY_IDENT);
        assert_eq!(
            &frame[1..9],
            &[0x01, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00]
        );
        assert_eq!(&frame[9..13], b"2.5+");
        assert!(frame[13..17].iter().all(|byte| *byte == b' '));
        assert_eq!(&frame[17..30], b"QEMU HARDDISK");
        assert!(frame[30..57].iter().all(|byte| *byte == b' '));
        assert!(frame[57..].iter().all(|byte| *byte == 0));
    }

    #[test]
    fn ident_encode_normalizes_non_printable_bytes() {
        let frame = Reply::Ident {
            sectors: 1,
            firmware_rev: "1\x01",
            model: "\x7Fdisk",
        }
        .encode()
        .unwrap();
        assert_eq!(&frame[9..11], b"1 ");
        assert_eq!(&frame[17..22], b" disk");
    }

    #[test]
    fn ident_encode_rejects_overlong_identity() {
        assert_eq!(
            Reply::Ident {
                sectors: 1,
                firmware_rev: &model_of_len(FIRMWARE_BYTES + 1),
                model: "x",
            }
            .encode(),
            Err(AtaError::Protocol)
        );
        assert_eq!(
            Reply::Ident {
                sectors: 1,
                firmware_rev: "x",
                model: &model_of_len(MODEL_BYTES + 1),
            }
            .encode(),
            Err(AtaError::Protocol)
        );
    }

    #[test]
    fn data_reply_pins_token_and_sectors_offsets() {
        let frame = Reply::Data {
            token: 0x0102_0304,
            sectors: MAX_SECTORS_PER_REQUEST as u16,
        }
        .encode()
        .unwrap();
        assert_eq!(frame[0], REPLY_DATA);
        assert_eq!(&frame[1..5], &[0x04, 0x03, 0x02, 0x01]);
        assert_eq!(&frame[5..7], &[0x80, 0x00]);
        assert!(frame[7..].iter().all(|byte| *byte == 0));
    }

    #[test]
    fn reply_all_variants_roundtrip() {
        let replies = [
            Reply::Ack,
            Reply::Err {
                error: AtaError::NotFound,
                error_register: 0,
                status: 0,
            },
            Reply::Err {
                error: AtaError::Denied,
                error_register: 0x1F,
                status: 0x51,
            },
            Reply::Ident {
                sectors: 7106,
                firmware_rev: "1.0",
                model: "QEMU HARDDISK",
            },
            Reply::Ident {
                sectors: u64::MAX,
                firmware_rev: "",
                model: &model_of_len(MODEL_BYTES),
            },
            Reply::Data {
                token: 11,
                sectors: 1,
            },
            Reply::Data {
                token: 0,
                sectors: MAX_SECTORS_PER_REQUEST as u16,
            },
        ];
        for reply in replies {
            assert_eq!(Reply::decode(&reply.encode().unwrap()), Ok(reply.clone()));
        }
    }

    #[test]
    fn reply_decode_rejects_unknown_tag() {
        for tag in [0x00u8, 0x0F, 0x14, 0xFF] {
            let mut frame = [0u8; FRAME_BYTES];
            frame[0] = tag;
            assert_eq!(Reply::decode(&frame), Err(AtaError::Protocol));
        }
    }

    #[test]
    fn reply_decode_rejects_nonzero_ack_tail() {
        let mut frame = Reply::Ack.encode().unwrap();
        frame[30] = 0x40;
        assert_eq!(Reply::decode(&frame), Err(AtaError::Protocol));
    }

    #[test]
    fn reply_decode_rejects_nonzero_error_tail() {
        let mut frame = Reply::Err {
            error: AtaError::Busy,
            error_register: 0,
            status: 0,
        }
        .encode()
        .unwrap();
        frame[4] = 1;
        assert_eq!(Reply::decode(&frame), Err(AtaError::Protocol));
    }

    #[test]
    fn reply_decode_rejects_nonzero_ident_tail() {
        let mut frame = Reply::Ident {
            sectors: 1,
            firmware_rev: "x",
            model: "y",
        }
        .encode()
        .unwrap();
        frame[57] = 1;
        assert_eq!(Reply::decode(&frame), Err(AtaError::Protocol));
    }

    #[test]
    fn reply_decode_rejects_nonzero_data_tail() {
        let mut frame = Reply::Data {
            token: 1,
            sectors: 1,
        }
        .encode()
        .unwrap();
        frame[7] = 1;
        assert_eq!(Reply::decode(&frame), Err(AtaError::Protocol));
    }

    #[test]
    fn reply_decode_rejects_data_sectors_out_of_bounds() {
        for sectors in [0u16, 129, u16::MAX] {
            let mut frame = Reply::Data {
                token: 1,
                sectors: 1,
            }
            .encode()
            .unwrap();
            frame[5..7].copy_from_slice(&sectors.to_le_bytes());
            assert_eq!(
                Reply::decode(&frame),
                Err(AtaError::Protocol),
                "sectors {sectors} accepted"
            );
        }
    }

    #[test]
    fn wire_error_codes_roundtrip() {
        let codes = [
            AtaError::NotFound,
            AtaError::BadAddress,
            AtaError::Busy,
            AtaError::Device,
            AtaError::Denied,
        ];
        for code in codes {
            assert_eq!(AtaError::from_byte(code.to_byte()), code);
        }
        assert_eq!(AtaError::NotFound.to_byte(), 1);
        assert_eq!(AtaError::Denied.to_byte(), 5);
        assert_eq!(AtaError::Protocol.to_byte(), 255);
    }

    #[test]
    fn unknown_wire_error_bytes_map_to_protocol() {
        for byte in [0u8, 6, 128, 254] {
            assert_eq!(AtaError::from_byte(byte), AtaError::Protocol);
        }
    }

    #[test]
    fn ident_decode_rejects_invalid_utf8_identity() {
        let mut frame = Reply::Ident {
            sectors: 1,
            firmware_rev: "ok",
            model: "ok",
        }
        .encode()
        .unwrap();
        frame[20] = 0xC3;
        frame[21] = 0x28;
        assert_eq!(Reply::decode(&frame), Err(AtaError::Protocol));
    }

    fn seed_frames() -> Vec<[u8; FRAME_BYTES]> {
        let requests = [
            Request::Identify { drive: 0 },
            Request::Read { drive: 1, lba: 7105, count: 1 },
            Request::Write { drive: 0, lba: 0, count: 128, token: 0x00AB_CDEF },
            Request::Flush { drive: 1 },
        ];
        let replies = [
            Reply::Ack,
            Reply::Err { error: AtaError::Device, error_register: 0x10, status: 0x41 },
            Reply::Ident { sectors: 7106, firmware_rev: "1.0", model: "QEMU HARDDISK" },
            Reply::Data { token: 9, sectors: 2 },
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
