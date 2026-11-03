pub const FRAME_BYTES: usize = 60;
pub const RPC_MAX_DATA_BYTES: usize = 64 * 1024;
pub const MAX_NAME_BYTES: usize = 57;
pub const SERVICE_NAME: &str = "VFS";

pub const MODE_WRITE: u8 = 1;
pub const MODE_CREATE: u8 = 2;

pub const OPEN: u8 = 1;
pub const READ: u8 = 2;
pub const WRITE: u8 = 3;
pub const CLOSE: u8 = 4;
pub const DELETE: u8 = 5;
pub const LIST: u8 = 6;
pub const STAT: u8 = 7;
pub const LIST_DIR: u8 = 8;
pub const MOUNT: u8 = 9;
pub const FORMAT: u8 = 10;

pub const MOUNT_DIR: &str = "hdd";

pub const REPLY_ACK: u8 = 0x10;
pub const REPLY_ERROR: u8 = 0x11;
pub const REPLY_EOF: u8 = 0x12;
pub const REPLY_OPENED: u8 = 0x13;
pub const REPLY_READ_DATA: u8 = 0x14;
pub const REPLY_WRITTEN: u8 = 0x15;
pub const REPLY_LISTED: u8 = 0x16;
pub const REPLY_STAT: u8 = 0x17;

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[repr(u8)]
pub enum EntityKind {
    File = 0,
    Directory = 1,
}

impl EntityKind {
    pub fn to_byte(self) -> u8 {
        self as u8
    }

    pub fn from_byte(byte: u8) -> Option<EntityKind> {
        match byte {
            0 => Some(EntityKind::File),
            1 => Some(EntityKind::Directory),
            _ => None,
        }
    }
}

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
    Busy = 8,
    NotFormatted = 9,
    Protocol = 255,
}

impl VfsError {
    pub fn to_byte(self) -> u8 {
        self as u8
    }

    pub fn from_byte(byte: u8) -> VfsError {
        match byte {
            1 => VfsError::NotFound,
            2 => VfsError::AlreadyExists,
            3 => VfsError::NotPermitted,
            4 => VfsError::BadHandle,
            5 => VfsError::InvalidPath,
            6 => VfsError::ListTooLarge,
            7 => VfsError::ServerResource,
            8 => VfsError::Busy,
            9 => VfsError::NotFormatted,
            _ => VfsError::Protocol,
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum Request<'a> {
    Open { mode: u8, name: &'a str },
    Read { handle: u32, offset: u64, count: u32 },
    Write { handle: u32, offset: u64, token: u32 },
    Close { handle: u32 },
    Delete { name: &'a str },
    List,
    Stat { name: &'a str },
    ListDir { dir: &'a str },
    Mount { drive: u8, dir: &'a str },
    Format { drive: u8 },
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum Reply {
    Ack,
    Err(VfsError),
    Eof,
    Opened { handle: u32 },
    ReadData { token: u32, count: u32 },
    Written { bytes: u32 },
    Listed { token: u32, total: u32 },
    Stat { kind: EntityKind, size: u64 },
}

pub fn valid_name(name: &str) -> bool {
    let bytes = name.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= MAX_NAME_BYTES
        && !bytes.contains(&0x0A)
        && !bytes.contains(&0x00)
        && !bytes.contains(&b'/')
        && name != "."
        && name != ".."
}

impl Request<'_> {
    pub fn encode(&self) -> Result<[u8; FRAME_BYTES], VfsError> {
        match self {
            Request::Open { mode, name } => {
                if !valid_name(name) {
                    return Err(VfsError::InvalidPath);
                }
                let mut frame = [0u8; FRAME_BYTES];
                frame[0] = OPEN;
                frame[1] = *mode;
                frame[2] = name.len() as u8;
                frame[3..3 + name.len()].copy_from_slice(name.as_bytes());
                Ok(frame)
            }
            Request::Read { handle, offset, count } => {
                let mut frame = [0u8; FRAME_BYTES];
                frame[0] = READ;
                frame[1..5].copy_from_slice(&handle.to_le_bytes());
                frame[5..13].copy_from_slice(&offset.to_le_bytes());
                frame[13..17].copy_from_slice(&count.to_le_bytes());
                Ok(frame)
            }
            Request::Write { handle, offset, token } => {
                let mut frame = [0u8; FRAME_BYTES];
                frame[0] = WRITE;
                frame[1..5].copy_from_slice(&handle.to_le_bytes());
                frame[5..13].copy_from_slice(&offset.to_le_bytes());
                frame[13..17].copy_from_slice(&token.to_le_bytes());
                Ok(frame)
            }
            Request::Close { handle } => {
                let mut frame = [0u8; FRAME_BYTES];
                frame[0] = CLOSE;
                frame[1..5].copy_from_slice(&handle.to_le_bytes());
                Ok(frame)
            }
            Request::Delete { name } => {
                if !valid_name(name) {
                    return Err(VfsError::InvalidPath);
                }
                let mut frame = [0u8; FRAME_BYTES];
                frame[0] = DELETE;
                frame[1] = name.len() as u8;
                frame[2..2 + name.len()].copy_from_slice(name.as_bytes());
                Ok(frame)
            }
            Request::List => {
                let mut frame = [0u8; FRAME_BYTES];
                frame[0] = LIST;
                Ok(frame)
            }
            Request::Stat { name } => {
                if !valid_name(name) {
                    return Err(VfsError::InvalidPath);
                }
                let mut frame = [0u8; FRAME_BYTES];
                frame[0] = STAT;
                frame[1] = name.len() as u8;
                frame[2..2 + name.len()].copy_from_slice(name.as_bytes());
                Ok(frame)
            }
            Request::ListDir { dir } => {
                if !valid_name(dir) {
                    return Err(VfsError::InvalidPath);
                }
                let mut frame = [0u8; FRAME_BYTES];
                frame[0] = LIST_DIR;
                frame[1] = dir.len() as u8;
                frame[2..2 + dir.len()].copy_from_slice(dir.as_bytes());
                Ok(frame)
            }
            Request::Mount { drive, dir } => {
                if !valid_name(dir) {
                    return Err(VfsError::InvalidPath);
                }
                let mut frame = [0u8; FRAME_BYTES];
                frame[0] = MOUNT;
                frame[1] = *drive;
                frame[2] = dir.len() as u8;
                frame[3..3 + dir.len()].copy_from_slice(dir.as_bytes());
                Ok(frame)
            }
            Request::Format { drive } => {
                let mut frame = [0u8; FRAME_BYTES];
                frame[0] = FORMAT;
                frame[1] = *drive;
                Ok(frame)
            }
        }
    }

    pub fn decode(frame: &[u8; FRAME_BYTES]) -> Result<Request<'_>, VfsError> {
        match frame[0] {
            OPEN => {
                let mode = frame[1];
                let name = decode_name(frame, 3, frame[2] as usize)?;
                Ok(Request::Open { mode, name })
            }
            READ => Ok(Request::Read {
                handle: read_u32(frame, 1),
                offset: read_u64(frame, 5),
                count: read_u32(frame, 13),
            }),
            WRITE => Ok(Request::Write {
                handle: read_u32(frame, 1),
                offset: read_u64(frame, 5),
                token: read_u32(frame, 13),
            }),
            CLOSE => Ok(Request::Close {
                handle: read_u32(frame, 1),
            }),
            DELETE => {
                let name = decode_name(frame, 2, frame[1] as usize)?;
                Ok(Request::Delete { name })
            }
            LIST => Ok(Request::List),
            STAT => {
                let name = decode_name(frame, 2, frame[1] as usize)?;
                Ok(Request::Stat { name })
            }
            LIST_DIR => {
                let dir = decode_name(frame, 2, frame[1] as usize)?;
                Ok(Request::ListDir { dir })
            }
            MOUNT => {
                let drive = frame[1];
                let dir = decode_name(frame, 3, frame[2] as usize)?;
                Ok(Request::Mount { drive, dir })
            }
            FORMAT => Ok(Request::Format { drive: frame[1] }),
            _ => Err(VfsError::Protocol),
        }
    }
}

fn decode_name<'a>(
    frame: &'a [u8; FRAME_BYTES],
    offset: usize,
    name_len: usize,
) -> Result<&'a str, VfsError> {
    if name_len == 0 || name_len > MAX_NAME_BYTES {
        return Err(VfsError::InvalidPath);
    }
    let end = offset + name_len;
    if end > FRAME_BYTES {
        return Err(VfsError::InvalidPath);
    }
    let name =
        core::str::from_utf8(&frame[offset..end]).map_err(|_| VfsError::InvalidPath)?;
    if !valid_name(name) {
        return Err(VfsError::InvalidPath);
    }
    Ok(name)
}

fn read_u32(frame: &[u8; FRAME_BYTES], at: usize) -> u32 {
    u32::from_le_bytes(frame[at..at + 4].try_into().unwrap())
}

fn read_u64(frame: &[u8; FRAME_BYTES], at: usize) -> u64 {
    u64::from_le_bytes(frame[at..at + 8].try_into().unwrap())
}

impl Reply {
    pub fn encode(&self) -> [u8; FRAME_BYTES] {
        let mut frame = [0u8; FRAME_BYTES];
        match self {
            Reply::Ack => frame[0] = REPLY_ACK,
            Reply::Err(error) => {
                frame[0] = REPLY_ERROR;
                frame[1] = error.to_byte();
            }
            Reply::Eof => frame[0] = REPLY_EOF,
            Reply::Opened { handle } => {
                frame[0] = REPLY_OPENED;
                frame[1..5].copy_from_slice(&handle.to_le_bytes());
            }
            Reply::ReadData { token, count } => {
                frame[0] = REPLY_READ_DATA;
                frame[1..5].copy_from_slice(&token.to_le_bytes());
                frame[5..9].copy_from_slice(&count.to_le_bytes());
            }
            Reply::Written { bytes } => {
                frame[0] = REPLY_WRITTEN;
                frame[1..5].copy_from_slice(&bytes.to_le_bytes());
            }
            Reply::Listed { token, total } => {
                frame[0] = REPLY_LISTED;
                frame[1..5].copy_from_slice(&token.to_le_bytes());
                frame[5..9].copy_from_slice(&total.to_le_bytes());
            }
            Reply::Stat { kind, size } => {
                frame[0] = REPLY_STAT;
                frame[1] = kind.to_byte();
                frame[2..10].copy_from_slice(&size.to_le_bytes());
            }
        }
        frame
    }

    pub fn decode(frame: &[u8; FRAME_BYTES]) -> Result<Reply, ()> {
        match frame[0] {
            REPLY_ACK => {
                zero_tail(frame, 1)?;
                Ok(Reply::Ack)
            }
            REPLY_ERROR => Ok(Reply::Err(VfsError::from_byte(frame[1]))),
            REPLY_EOF => {
                zero_tail(frame, 1)?;
                Ok(Reply::Eof)
            }
            REPLY_OPENED => {
                let handle = read_u32(frame, 1);
                if handle == 0 {
                    return Err(());
                }
                Ok(Reply::Opened { handle })
            }
            REPLY_READ_DATA => {
                let token = read_u32(frame, 1);
                let count = read_u32(frame, 5);
                if count == 0 {
                    return Err(());
                }
                Ok(Reply::ReadData { token, count })
            }
            REPLY_WRITTEN => Ok(Reply::Written { bytes: read_u32(frame, 1) }),
            REPLY_LISTED => {
                let token = read_u32(frame, 1);
                let total = read_u32(frame, 5);
                if total == 0 {
                    return Err(());
                }
                Ok(Reply::Listed { token, total })
            }
            REPLY_STAT => {
                let kind = EntityKind::from_byte(frame[1]).ok_or(())?;
                Ok(Reply::Stat {
                    kind,
                    size: read_u64(frame, 2),
                })
            }
            _ => Err(()),
        }
    }
}

fn zero_tail(frame: &[u8; FRAME_BYTES], from: usize) -> Result<(), ()> {
    if frame[from..].iter().any(|byte| *byte != 0) {
        Err(())
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn name_of_len(length: usize) -> alloc::string::String {
        (0..length).map(|_| 'k').collect()
    }

    #[test]
    fn open_frame_pins_offsets_for_short_name() {
        let frame = Request::Open { mode: MODE_WRITE | MODE_CREATE, name: "a.txt" }
            .encode()
            .unwrap();
        assert_eq!(frame[0], OPEN);
        assert_eq!(frame[1], 3);
        assert_eq!(frame[2], 5);
        assert_eq!(&frame[3..8], b"a.txt");
        assert!(frame[8..].iter().all(|byte| *byte == 0));
    }

    #[test]
    fn delete_frame_pins_offsets_for_short_name() {
        let frame = Request::Delete { name: "a.txt" }.encode().unwrap();
        assert_eq!(frame[0], DELETE);
        assert_eq!(frame[1], 5);
        assert_eq!(&frame[2..7], b"a.txt");
        assert!(frame[7..].iter().all(|byte| *byte == 0));
    }

    #[test]
    fn read_frame_pins_little_endian_offsets() {
        let frame = Request::Read {
            handle: 0x0102_0304,
            offset: 0x0807_0605_0403_0201,
            count: 0x0001_0000,
        }
        .encode()
        .unwrap();
        assert_eq!(frame[0], READ);
        assert_eq!(frame[1], 0x04);
        assert_eq!(frame[2], 0x03);
        assert_eq!(frame[3], 0x02);
        assert_eq!(frame[4], 0x01);
        assert_eq!(frame[5], 0x01);
        assert_eq!(frame[6], 0x02);
        assert_eq!(frame[7], 0x03);
        assert_eq!(frame[8], 0x04);
        assert_eq!(frame[9], 0x05);
        assert_eq!(frame[10], 0x06);
        assert_eq!(frame[11], 0x07);
        assert_eq!(frame[12], 0x08);
        assert_eq!(frame[13], 0x00);
        assert_eq!(frame[14], 0x00);
        assert_eq!(frame[15], 0x01);
        assert_eq!(frame[16], 0x00);
        assert!(frame[17..].iter().all(|byte| *byte == 0));
    }

    #[test]
    fn write_frame_pins_little_endian_offsets() {
        let frame = Request::Write {
            handle: 0x0A0B_0C0D,
            offset: 0x1122_3344_5566_7788,
            token: 0x00AB_CDEF,
        }
        .encode()
        .unwrap();
        assert_eq!(frame[0], WRITE);
        assert_eq!(&frame[1..5], &[0x0D, 0x0C, 0x0B, 0x0A]);
        assert_eq!(
            &frame[5..13],
            &[0x88, 0x77, 0x66, 0x55, 0x44, 0x33, 0x22, 0x11]
        );
        assert_eq!(&frame[13..17], &[0xEF, 0xCD, 0xAB, 0x00]);
        assert!(frame[17..].iter().all(|byte| *byte == 0));
    }

    #[test]
    fn close_frame_pins_offsets() {
        let frame = Request::Close { handle: 0x0000_0007 }.encode().unwrap();
        assert_eq!(frame[0], CLOSE);
        assert_eq!(&frame[1..5], &[0x07, 0x00, 0x00, 0x00]);
        assert!(frame[5..].iter().all(|byte| *byte == 0));
    }

    #[test]
    fn list_frame_uses_op_byte_only() {
        let frame = Request::List.encode().unwrap();
        assert_eq!(frame[0], LIST);
        assert!(frame[1..].iter().all(|byte| *byte == 0));
    }

    #[test]
    fn open_request_roundtrips() {
        let frame = Request::Open { mode: MODE_WRITE, name: "notes.txt" }.encode().unwrap();
        assert_eq!(
            Request::decode(&frame),
            Ok(Request::Open {
                mode: MODE_WRITE,
                name: "notes.txt"
            })
        );
    }

    #[test]
    fn open_request_roundtrips_maximum_name() {
        let name = name_of_len(MAX_NAME_BYTES);
        let frame = Request::Open { mode: 0, name: &name }.encode().unwrap();
        assert_eq!(Request::decode(&frame), Ok(Request::Open { mode: 0, name: &name }));
    }

    #[test]
    fn read_request_roundtrips() {
        let frame = Request::Read {
            handle: 9,
            offset: 0xFFFF_FFFF_FFFF_FF00,
            count: 65536,
        }
        .encode()
        .unwrap();
        assert_eq!(
            Request::decode(&frame),
            Ok(Request::Read {
                handle: 9,
                offset: 0xFFFF_FFFF_FFFF_FF00,
                count: 65536
            })
        );
    }

    #[test]
    fn write_request_roundtrips() {
        let frame = Request::Write {
            handle: 3,
            offset: 4096,
            token: 0x0001_0000,
        }
        .encode()
        .unwrap();
        assert_eq!(
            Request::decode(&frame),
            Ok(Request::Write {
                handle: 3,
                offset: 4096,
                token: 0x0001_0000
            })
        );
    }

    #[test]
    fn close_request_roundtrips() {
        let frame = Request::Close { handle: 1 }.encode().unwrap();
        assert_eq!(Request::decode(&frame), Ok(Request::Close { handle: 1 }));
    }

    #[test]
    fn delete_request_roundtrips() {
        let frame = Request::Delete { name: "blob.bin" }.encode().unwrap();
        assert_eq!(
            Request::decode(&frame),
            Ok(Request::Delete { name: "blob.bin" })
        );
    }

    #[test]
    fn list_request_roundtrips() {
        assert_eq!(Request::decode(&Request::List.encode().unwrap()), Ok(Request::List));
    }

    #[test]
    fn stat_frame_pins_offsets_for_short_name() {
        let frame = Request::Stat { name: "a.txt" }.encode().unwrap();
        assert_eq!(frame[0], STAT);
        assert_eq!(frame[1], 5);
        assert_eq!(&frame[2..7], b"a.txt");
        assert!(frame[7..].iter().all(|byte| *byte == 0));
    }

    #[test]
    fn stat_request_roundtrips() {
        let frame = Request::Stat { name: "notes.txt" }.encode().unwrap();
        assert_eq!(
            Request::decode(&frame),
            Ok(Request::Stat { name: "notes.txt" })
        );
    }

    #[test]
    fn stat_request_roundtrips_maximum_name() {
        let name = name_of_len(MAX_NAME_BYTES);
        let frame = Request::Stat { name: &name }.encode().unwrap();
        assert_eq!(Request::decode(&frame), Ok(Request::Stat { name: &name }));
    }

    #[test]
    fn list_dir_frame_pins_offsets_for_mount_dir() {
        let frame = Request::ListDir { dir: MOUNT_DIR }.encode().unwrap();
        assert_eq!(frame[0], LIST_DIR);
        assert_eq!(frame[1], 3);
        assert_eq!(&frame[2..5], b"hdd");
        assert!(frame[5..].iter().all(|byte| *byte == 0));
    }

    #[test]
    fn list_dir_request_roundtrips() {
        let frame = Request::ListDir { dir: MOUNT_DIR }.encode().unwrap();
        assert_eq!(Request::decode(&frame), Ok(Request::ListDir { dir: MOUNT_DIR }));
    }

    #[test]
    fn list_dir_rejects_invalid_dir_names() {
        assert_eq!(Request::ListDir { dir: "" }.encode(), Err(VfsError::InvalidPath));
        assert_eq!(
            Request::ListDir { dir: "/hdd" }.encode(),
            Err(VfsError::InvalidPath)
        );
        let mut frame = [0u8; FRAME_BYTES];
        frame[0] = LIST_DIR;
        frame[1] = 3;
        frame[2..5].copy_from_slice(b"a/b");
        assert_eq!(Request::decode(&frame), Err(VfsError::InvalidPath));
    }

    #[test]
    fn mount_frame_pins_drive_and_dir_offsets() {
        let frame = Request::Mount { drive: 1, dir: MOUNT_DIR }.encode().unwrap();
        assert_eq!(frame[0], MOUNT);
        assert_eq!(frame[1], 1);
        assert_eq!(frame[2], 3);
        assert_eq!(&frame[3..6], b"hdd");
        assert!(frame[6..].iter().all(|byte| *byte == 0));
    }

    #[test]
    fn mount_request_roundtrips() {
        let frame = Request::Mount { drive: 0, dir: MOUNT_DIR }.encode().unwrap();
        assert_eq!(
            Request::decode(&frame),
            Ok(Request::Mount { drive: 0, dir: MOUNT_DIR })
        );
    }

    #[test]
    fn mount_rejects_invalid_dir_names() {
        assert_eq!(
            Request::Mount { drive: 1, dir: "" }.encode(),
            Err(VfsError::InvalidPath)
        );
        assert_eq!(
            Request::Mount { drive: 1, dir: "bad\nname" }.encode(),
            Err(VfsError::InvalidPath)
        );
    }

    #[test]
    fn format_frame_pins_drive_offset() {
        let frame = Request::Format { drive: 1 }.encode().unwrap();
        assert_eq!(frame[0], FORMAT);
        assert_eq!(frame[1], 1);
        assert!(frame[2..].iter().all(|byte| *byte == 0));
    }

    #[test]
    fn format_request_roundtrips() {
        let frame = Request::Format { drive: 0 }.encode().unwrap();
        assert_eq!(Request::decode(&frame), Ok(Request::Format { drive: 0 }));
    }

    #[test]
    fn decode_stat_rejects_invalid_names() {
        let mut nested = [0u8; FRAME_BYTES];
        nested[0] = STAT;
        nested[1] = 3;
        nested[2..5].copy_from_slice(b"a/b");
        assert_eq!(Request::decode(&nested), Err(VfsError::InvalidPath));
        let mut dot = [0u8; FRAME_BYTES];
        dot[0] = STAT;
        dot[1] = 1;
        dot[2] = b'.';
        assert_eq!(Request::decode(&dot), Err(VfsError::InvalidPath));
        let mut nul = [0u8; FRAME_BYTES];
        nul[0] = STAT;
        nul[1] = 3;
        nul[2] = b'a';
        nul[4] = b'b';
        assert_eq!(Request::decode(&nul), Err(VfsError::InvalidPath));
    }

    #[test]
    fn decode_request_rejects_unknown_op_bytes() {
        for op in [0u8, 11, 255] {
            let mut frame = Request::List.encode().unwrap();
            frame[0] = op;
            assert_eq!(
                Request::decode(&frame),
                Err(VfsError::Protocol),
                "op byte {op} accepted"
            );
        }
    }

    #[test]
    fn decode_request_rejects_zero_name_len() {
        let mut open_frame = Request::Open { mode: 0, name: "x" }.encode().unwrap();
        open_frame[2] = 0;
        assert_eq!(Request::decode(&open_frame), Err(VfsError::InvalidPath));
        let mut delete_frame = Request::Delete { name: "x" }.encode().unwrap();
        delete_frame[1] = 0;
        assert_eq!(Request::decode(&delete_frame), Err(VfsError::InvalidPath));
        let mut stat_frame = Request::Stat { name: "x" }.encode().unwrap();
        stat_frame[1] = 0;
        assert_eq!(Request::decode(&stat_frame), Err(VfsError::InvalidPath));
    }

    #[test]
    fn decode_request_rejects_name_len_beyond_limit() {
        let mut open_frame = Request::Open { mode: 0, name: "x" }.encode().unwrap();
        open_frame[2] = 58;
        assert_eq!(Request::decode(&open_frame), Err(VfsError::InvalidPath));
        let mut delete_frame = Request::Delete { name: "x" }.encode().unwrap();
        delete_frame[1] = 58;
        assert_eq!(Request::decode(&delete_frame), Err(VfsError::InvalidPath));
        let mut stat_frame = Request::Stat { name: "x" }.encode().unwrap();
        stat_frame[1] = 58;
        assert_eq!(Request::decode(&stat_frame), Err(VfsError::InvalidPath));
    }

    #[test]
    fn decode_request_rejects_declared_name_past_frame_end() {
        let mut frame = [0u8; FRAME_BYTES];
        frame[0] = OPEN;
        frame[2] = 255;
        assert_eq!(Request::decode(&frame), Err(VfsError::InvalidPath));
        let mut delete_frame = [0u8; FRAME_BYTES];
        delete_frame[0] = DELETE;
        delete_frame[1] = 255;
        assert_eq!(Request::decode(&delete_frame), Err(VfsError::InvalidPath));
        let mut stat_frame = [0u8; FRAME_BYTES];
        stat_frame[0] = STAT;
        stat_frame[1] = 255;
        assert_eq!(Request::decode(&stat_frame), Err(VfsError::InvalidPath));
    }

    #[test]
    fn decode_request_rejects_invalid_utf8_name() {
        let mut frame = Request::Open { mode: 0, name: "abc" }.encode().unwrap();
        frame[4] = 0xFF;
        assert_eq!(Request::decode(&frame), Err(VfsError::InvalidPath));
    }

    #[test]
    fn decode_open_rejects_path_separator_name() {
        let mut frame = [0u8; FRAME_BYTES];
        frame[0] = OPEN;
        frame[2] = 3;
        frame[3..6].copy_from_slice(b"a/b");
        assert_eq!(Request::decode(&frame), Err(VfsError::InvalidPath));
    }

    #[test]
    fn decode_open_rejects_dot_and_dotdot_names() {
        let mut dot = [0u8; FRAME_BYTES];
        dot[0] = OPEN;
        dot[2] = 1;
        dot[3] = b'.';
        assert_eq!(Request::decode(&dot), Err(VfsError::InvalidPath));
        let mut dotdot = [0u8; FRAME_BYTES];
        dotdot[0] = OPEN;
        dotdot[2] = 2;
        dotdot[3..5].copy_from_slice(b"..");
        assert_eq!(Request::decode(&dotdot), Err(VfsError::InvalidPath));
    }

    #[test]
    fn decode_delete_rejects_dot_and_path_separator_names() {
        let mut dot = [0u8; FRAME_BYTES];
        dot[0] = DELETE;
        dot[1] = 1;
        dot[2] = b'.';
        assert_eq!(Request::decode(&dot), Err(VfsError::InvalidPath));
        let mut nested = [0u8; FRAME_BYTES];
        nested[0] = DELETE;
        nested[1] = 3;
        nested[2..5].copy_from_slice(b"a/b");
        assert_eq!(Request::decode(&nested), Err(VfsError::InvalidPath));
    }

    #[test]
    fn decode_open_rejects_newline_and_nul_in_name() {
        let mut newline = [0u8; FRAME_BYTES];
        newline[0] = OPEN;
        newline[2] = 3;
        newline[3..6].copy_from_slice(b"a\nb");
        assert_eq!(Request::decode(&newline), Err(VfsError::InvalidPath));
        let mut nul = [0u8; FRAME_BYTES];
        nul[0] = OPEN;
        nul[2] = 3;
        nul[3] = b'a';
        nul[4] = 0x00;
        nul[5] = b'b';
        assert_eq!(Request::decode(&nul), Err(VfsError::InvalidPath));
    }

    #[test]
    fn decode_request_unknown_op_outranks_bad_name() {
        let mut frame = Request::Delete { name: "x" }.encode().unwrap();
        frame[0] = 200;
        frame[1] = 255;
        assert_eq!(Request::decode(&frame), Err(VfsError::Protocol));
    }

    #[test]
    fn valid_name_accepts_one_to_fifty_seven_bytes() {
        assert!(valid_name("a"));
        assert!(valid_name(&name_of_len(MAX_NAME_BYTES)));
    }

    #[test]
    fn valid_name_rejects_empty_and_oversize_and_forbidden_bytes() {
        assert!(!valid_name(""));
        assert!(!valid_name(&name_of_len(MAX_NAME_BYTES + 1)));
        assert!(!valid_name("bad\nname"));
        assert!(!valid_name("bad\0name"));
        assert!(!valid_name("bad/name"));
        assert!(!valid_name("/"));
        assert!(!valid_name("/abs"));
        assert!(!valid_name("."));
        assert!(!valid_name(".."));
    }

    #[test]
    fn valid_name_accepts_dots_inside_names() {
        assert!(valid_name("..."));
        assert!(valid_name("notes.txt"));
        assert!(valid_name("a..b"));
        assert!(valid_name(".hidden"));
    }

    #[test]
    fn request_encode_rejects_invalid_names() {
        assert_eq!(Request::Open { mode: 0, name: "" }.encode(), Err(VfsError::InvalidPath));
        assert_eq!(
            Request::Open { mode: 0, name: &name_of_len(MAX_NAME_BYTES + 1) }.encode(),
            Err(VfsError::InvalidPath)
        );
        assert_eq!(
            Request::Open { mode: 0, name: "bad\nname" }.encode(),
            Err(VfsError::InvalidPath)
        );
        assert_eq!(
            Request::Open { mode: 0, name: "bad\0name" }.encode(),
            Err(VfsError::InvalidPath)
        );
        assert_eq!(Request::Delete { name: "" }.encode(), Err(VfsError::InvalidPath));
        assert_eq!(
            Request::Delete { name: "bad\nname" }.encode(),
            Err(VfsError::InvalidPath)
        );
        assert_eq!(Request::Stat { name: "" }.encode(), Err(VfsError::InvalidPath));
        assert_eq!(
            Request::Stat { name: "bad/name" }.encode(),
            Err(VfsError::InvalidPath)
        );
        assert_eq!(Request::Stat { name: ".." }.encode(), Err(VfsError::InvalidPath));
    }

    #[test]
    fn wire_error_codes_roundtrip() {
        let codes = [
            VfsError::NotFound,
            VfsError::AlreadyExists,
            VfsError::NotPermitted,
            VfsError::BadHandle,
            VfsError::InvalidPath,
            VfsError::ListTooLarge,
            VfsError::ServerResource,
            VfsError::Busy,
            VfsError::NotFormatted,
        ];
        for code in codes {
            assert_eq!(VfsError::from_byte(code.to_byte()), code);
        }
        assert_eq!(VfsError::NotFound.to_byte(), 1);
        assert_eq!(VfsError::ServerResource.to_byte(), 7);
        assert_eq!(VfsError::Busy.to_byte(), 8);
        assert_eq!(VfsError::NotFormatted.to_byte(), 9);
        assert_eq!(VfsError::Protocol.to_byte(), 255);
    }

    #[test]
    fn unknown_wire_error_bytes_map_to_protocol() {
        for byte in [0u8, 10, 200, 254] {
            assert_eq!(VfsError::from_byte(byte), VfsError::Protocol);
        }
    }

    #[test]
    fn reply_tag_constants_are_pinned() {
        assert_eq!(REPLY_ACK, 0x10);
        assert_eq!(REPLY_ERROR, 0x11);
        assert_eq!(REPLY_EOF, 0x12);
        assert_eq!(REPLY_OPENED, 0x13);
        assert_eq!(REPLY_READ_DATA, 0x14);
        assert_eq!(REPLY_WRITTEN, 0x15);
        assert_eq!(REPLY_LISTED, 0x16);
        assert_eq!(REPLY_STAT, 0x17);
        assert_eq!(OPEN, 1);
        assert_eq!(READ, 2);
        assert_eq!(WRITE, 3);
        assert_eq!(CLOSE, 4);
        assert_eq!(DELETE, 5);
        assert_eq!(LIST, 6);
        assert_eq!(STAT, 7);
        assert_eq!(LIST_DIR, 8);
        assert_eq!(MOUNT, 9);
        assert_eq!(FORMAT, 10);
        assert_eq!(MOUNT_DIR, "hdd");
        assert!(valid_name(MOUNT_DIR));
        assert_eq!(EntityKind::File.to_byte(), 0);
        assert_eq!(EntityKind::Directory.to_byte(), 1);
        let opcodes = [OPEN, READ, WRITE, CLOSE, DELETE, LIST, STAT, LIST_DIR, MOUNT, FORMAT];
        let tags = [
            REPLY_ACK,
            REPLY_ERROR,
            REPLY_EOF,
            REPLY_OPENED,
            REPLY_READ_DATA,
            REPLY_WRITTEN,
            REPLY_LISTED,
            REPLY_STAT,
        ];
        for tag in tags {
            assert!(!opcodes.contains(&tag), "tag {tag:#04x} collides with opcode");
        }
    }

    #[test]
    fn ack_reply_pins_tag_and_zero_tail() {
        let frame = Reply::Ack.encode();
        assert_eq!(frame[0], REPLY_ACK);
        assert!(frame[1..].iter().all(|byte| *byte == 0));
    }

    #[test]
    fn err_reply_pins_tag_and_error_byte() {
        let frame = Reply::Err(VfsError::BadHandle).encode();
        assert_eq!(frame[0], REPLY_ERROR);
        assert_eq!(frame[1], 4);
        let protocol = Reply::Err(VfsError::Protocol).encode();
        assert_eq!(protocol[0], REPLY_ERROR);
        assert_eq!(protocol[1], 255);
    }

    #[test]
    fn eof_reply_pins_tag_and_zero_tail() {
        let frame = Reply::Eof.encode();
        assert_eq!(frame[0], REPLY_EOF);
        assert!(frame[1..].iter().all(|byte| *byte == 0));
    }

    #[test]
    fn opened_reply_pins_tag_and_handle_offset() {
        let frame = Reply::Opened { handle: 0x0102_0304 }.encode();
        assert_eq!(frame[0], REPLY_OPENED);
        assert_eq!(&frame[1..5], &[0x04, 0x03, 0x02, 0x01]);
        assert!(frame[5..].iter().all(|byte| *byte == 0));
    }

    #[test]
    fn read_data_reply_pins_tag_and_field_offsets() {
        let frame = Reply::ReadData {
            token: 0x0000_0100,
            count: 0x0000_2000,
        }
        .encode();
        assert_eq!(frame[0], REPLY_READ_DATA);
        assert_eq!(&frame[1..5], &[0x00, 0x01, 0x00, 0x00]);
        assert_eq!(&frame[5..9], &[0x00, 0x20, 0x00, 0x00]);
        assert!(frame[9..].iter().all(|byte| *byte == 0));
    }

    #[test]
    fn written_reply_pins_tag_and_bytes_offset() {
        let frame = Reply::Written { bytes: 1024 }.encode();
        assert_eq!(frame[0], REPLY_WRITTEN);
        assert_eq!(&frame[1..5], &[0x00, 0x04, 0x00, 0x00]);
        assert!(frame[5..].iter().all(|byte| *byte == 0));
    }

    #[test]
    fn listed_reply_pins_tag_and_total_offset() {
        let frame = Reply::Listed {
            token: 0x0102_0304,
            total: 0x0000_1D6A,
        }
        .encode();
        assert_eq!(frame[0], REPLY_LISTED);
        assert_eq!(&frame[1..5], &[0x04, 0x03, 0x02, 0x01]);
        assert_eq!(&frame[5..9], &[0x6A, 0x1D, 0x00, 0x00]);
        assert!(frame[9..].iter().all(|byte| *byte == 0));
    }

    #[test]
    fn stat_reply_pins_tag_kind_and_size_offset() {
        let file = Reply::Stat {
            kind: EntityKind::File,
            size: 0x0102_0304_0506_0708,
        }
        .encode();
        assert_eq!(file[0], REPLY_STAT);
        assert_eq!(file[1], 0);
        assert_eq!(
            &file[2..10],
            &[0x08, 0x07, 0x06, 0x05, 0x04, 0x03, 0x02, 0x01]
        );
        assert!(file[10..].iter().all(|byte| *byte == 0));
        let dir = Reply::Stat {
            kind: EntityKind::Directory,
            size: 4096,
        }
        .encode();
        assert_eq!(dir[0], REPLY_STAT);
        assert_eq!(dir[1], 1);
        assert_eq!(&dir[2..10], &[0x00, 0x10, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]);
    }

    #[test]
    fn entity_kind_bytes_roundtrip_and_reject_unknown() {
        for kind in [EntityKind::File, EntityKind::Directory] {
            assert_eq!(EntityKind::from_byte(kind.to_byte()), Some(kind));
        }
        for byte in [2u8, 3, 128, 255] {
            assert_eq!(EntityKind::from_byte(byte), None, "byte {byte} accepted");
        }
    }

    #[test]
    fn reply_decode_rejects_unknown_entity_kind_byte() {
        let mut frame = Reply::Stat {
            kind: EntityKind::File,
            size: 1,
        }
        .encode();
        frame[1] = 2;
        assert_eq!(Reply::decode(&frame), Err(()));
    }

    #[test]
    fn stat_reply_accepts_zero_size_empty_file() {
        let frame = Reply::Stat {
            kind: EntityKind::File,
            size: 0,
        }
        .encode();
        assert_eq!(
            Reply::decode(&frame),
            Ok(Reply::Stat {
                kind: EntityKind::File,
                size: 0,
            })
        );
    }

    #[test]
    fn reply_all_variants_roundtrip() {
        let replies = [
            Reply::Ack,
            Reply::Err(VfsError::NotFound),
            Reply::Eof,
            Reply::Opened { handle: 7 },
            Reply::ReadData {
                token: 11,
                count: 4096,
            },
            Reply::Written { bytes: 1024 },
            Reply::Listed { token: 5, total: 9 },
            Reply::Stat {
                kind: EntityKind::File,
                size: 0,
            },
            Reply::Stat {
                kind: EntityKind::Directory,
                size: 0xFFFF_FFFF_FFFF_FF00,
            },
        ];
        for reply in replies {
            assert_eq!(Reply::decode(&reply.encode()), Ok(reply));
        }
    }

    #[test]
    fn reply_decode_rejects_unknown_tag() {
        for tag in [0x00u8, 0x0F, 0x18, 0xFF] {
            let mut frame = [0u8; FRAME_BYTES];
            frame[0] = tag;
            assert_eq!(Reply::decode(&frame), Err(()), "tag {tag:#04x} accepted");
        }
    }

    #[test]
    fn reply_decode_rejects_nonzero_ack_tail() {
        let mut frame = Reply::Ack.encode();
        frame[7] = 0x40;
        assert_eq!(Reply::decode(&frame), Err(()));
    }

    #[test]
    fn reply_decode_rejects_nonzero_eof_tail() {
        let mut frame = Reply::Eof.encode();
        frame[9] = 1;
        assert_eq!(Reply::decode(&frame), Err(()));
    }

    #[test]
    fn reply_decode_rejects_zero_handle() {
        let mut frame = [0u8; FRAME_BYTES];
        frame[0] = REPLY_OPENED;
        assert_eq!(Reply::decode(&frame), Err(()));
    }

    #[test]
    fn reply_decode_rejects_zero_count() {
        let mut frame = [0u8; FRAME_BYTES];
        frame[0] = REPLY_READ_DATA;
        frame[1..5].copy_from_slice(&7u32.to_le_bytes());
        assert_eq!(Reply::decode(&frame), Err(()));
    }

    #[test]
    fn reply_decode_rejects_zero_total() {
        let mut frame = [0u8; FRAME_BYTES];
        frame[0] = REPLY_LISTED;
        frame[1..5].copy_from_slice(&123u32.to_le_bytes());
        assert_eq!(Reply::decode(&frame), Err(()));
    }

    #[test]
    fn reply_decode_accepts_zero_bytes_written() {
        let frame = Reply::Written { bytes: 0 }.encode();
        assert_eq!(Reply::decode(&frame), Ok(Reply::Written { bytes: 0 }));
    }

    #[test]
    fn reply_decode_accepts_zero_read_token() {
        let frame = Reply::ReadData { token: 0, count: 1 }.encode();
        assert_eq!(
            Reply::decode(&frame),
            Ok(Reply::ReadData { token: 0, count: 1 })
        );
    }

    #[test]
    fn reply_decode_accepts_err_under_protocol_byte() {
        let mut frame = [0u8; FRAME_BYTES];
        frame[0] = REPLY_ERROR;
        frame[1] = 255;
        assert_eq!(Reply::decode(&frame), Ok(Reply::Err(VfsError::Protocol)));
    }
}
