use crate::wire::BlockError;
use alloc::string::String;
use alloc::vec::Vec;

pub const SERVICE_NAME: &str = "BLOCKD";

pub const FRAME_BYTES: usize = 60;
pub const NAME_BYTES: usize = 24;
pub const SERVICE_STRING_BUDGET: usize = FRAME_BYTES - 2;
pub const DESCRIPTION_BYTES: usize = 44;
pub const LIST_ENTRY_BYTES: usize = 112;

pub const REGISTER: u8 = 0x01;
pub const LIST: u8 = 0x02;
pub const PROBE: u8 = 0x03;
pub const READ: u8 = 0x04;
pub const WRITE: u8 = 0x05;
pub const FLUSH: u8 = 0x06;

pub const REPLY_ACK: u8 = 0x10;
pub const REPLY_ERROR: u8 = 0x11;
pub const REPLY_PROBE: u8 = 0x12;
pub const REPLY_DATA: u8 = 0x13;

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[repr(u8)]
pub enum DeviceKind {
    Disk = 0,
    Partition = 1,
}

impl DeviceKind {
    pub fn to_byte(self) -> u8 {
        self as u8
    }

    pub fn from_byte(byte: u8) -> Option<DeviceKind> {
        match byte {
            0 => Some(DeviceKind::Disk),
            1 => Some(DeviceKind::Partition),
            _ => None,
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[repr(u8)]
pub enum DeviceState {
    Clean = 0,
    Partitioned = 1,
    Corrupt = 2,
}

impl DeviceState {
    pub fn to_byte(self) -> u8 {
        self as u8
    }

    pub fn from_byte(byte: u8) -> Option<DeviceState> {
        match byte {
            0 => Some(DeviceState::Clean),
            1 => Some(DeviceState::Partitioned),
            2 => Some(DeviceState::Corrupt),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            DeviceState::Clean => "clean",
            DeviceState::Partitioned => "partitioned",
            DeviceState::Corrupt => "corrupt",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceInfo {
    pub name: String,
    pub source: String,
    pub description: String,
    pub block_count: u64,
    pub block_size: u32,
    pub kind: DeviceKind,
    pub state: DeviceState,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum Request<'a> {
    Register { service: &'a str },
    List,
    Probe { name: &'a str },
    Read { name: &'a str, block: u64, count: u16 },
    Write { name: &'a str, block: u64, count: u16, token: u32 },
    Flush { name: &'a str },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reply<'a> {
    Ack,
    Err(BlockError),
    Probe {
        block_count: u64,
        block_size: u32,
        kind: DeviceKind,
        state: DeviceState,
        description: &'a str,
    },
    Data {
        token: u32,
    },
}

pub fn valid_device_name(name: &str) -> bool {
    let bytes = name.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= NAME_BYTES
        && bytes[0] != b'-'
        && bytes
            .iter()
            .all(|byte| matches!(byte, b'a'..=b'z' | b'0'..=b'9' | b'-'))
}

pub fn valid_service(service: &str) -> bool {
    let bytes = service.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= SERVICE_STRING_BUDGET
        && bytes.iter().all(|byte| (0x21..=0x7E).contains(byte))
}

fn valid_source(source: &str) -> bool {
    let bytes = source.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= NAME_BYTES
        && bytes.iter().all(|byte| (0x21..=0x7E).contains(byte))
}

fn valid_count(count: u16) -> bool {
    count >= 1 && count as usize <= crate::wire::MAX_BLOCKS_PER_REQUEST
}

fn valid_block_size(block_size: u32) -> bool {
    block_size.is_power_of_two()
}

fn decode_name<'a>(frame: &'a [u8; FRAME_BYTES], name_len: usize) -> Result<&'a str, BlockError> {
    if name_len == 0 || name_len > NAME_BYTES {
        return Err(BlockError::Protocol);
    }
    let text = core::str::from_utf8(&frame[2..2 + name_len]).map_err(|_| BlockError::Protocol)?;
    if !valid_device_name(text) {
        return Err(BlockError::Protocol);
    }
    Ok(text)
}

fn put_padded(field: &mut [u8], text: &[u8]) {
    for (offset, byte) in field.iter_mut().enumerate() {
        *byte = match text.get(offset) {
            Some(byte) if (0x20..=0x7E).contains(byte) => *byte,
            _ => b' ',
        };
    }
}

impl Request<'_> {
    pub fn encode(&self) -> Result<[u8; FRAME_BYTES], BlockError> {
        let mut frame = [0u8; FRAME_BYTES];
        match self {
            Request::Register { service } => {
                if !valid_service(service) {
                    return Err(BlockError::Protocol);
                }
                frame[0] = REGISTER;
                frame[1] = service.len() as u8;
                frame[2..2 + service.len()].copy_from_slice(service.as_bytes());
            }
            Request::List => frame[0] = LIST,
            Request::Probe { name } => {
                if !valid_device_name(name) {
                    return Err(BlockError::Protocol);
                }
                frame[0] = PROBE;
                frame[1] = name.len() as u8;
                frame[2..2 + name.len()].copy_from_slice(name.as_bytes());
            }
            Request::Read { name, block, count } => {
                if !valid_device_name(name) || !valid_count(*count) {
                    return Err(BlockError::Protocol);
                }
                frame[0] = READ;
                frame[1] = name.len() as u8;
                frame[2..2 + name.len()].copy_from_slice(name.as_bytes());
                frame[26..34].copy_from_slice(&block.to_le_bytes());
                frame[34..36].copy_from_slice(&count.to_le_bytes());
            }
            Request::Write {
                name,
                block,
                count,
                token,
            } => {
                if !valid_device_name(name) || !valid_count(*count) {
                    return Err(BlockError::Protocol);
                }
                frame[0] = WRITE;
                frame[1] = name.len() as u8;
                frame[2..2 + name.len()].copy_from_slice(name.as_bytes());
                frame[26..34].copy_from_slice(&block.to_le_bytes());
                frame[34..36].copy_from_slice(&count.to_le_bytes());
                frame[36..40].copy_from_slice(&token.to_le_bytes());
            }
            Request::Flush { name } => {
                if !valid_device_name(name) {
                    return Err(BlockError::Protocol);
                }
                frame[0] = FLUSH;
                frame[1] = name.len() as u8;
                frame[2..2 + name.len()].copy_from_slice(name.as_bytes());
            }
        }
        Ok(frame)
    }

    pub fn decode(frame: &[u8; FRAME_BYTES]) -> Result<Request<'_>, BlockError> {
        match frame[0] {
            REGISTER => {
                let service_len = frame[1] as usize;
                if service_len == 0 || service_len > SERVICE_STRING_BUDGET {
                    return Err(BlockError::Protocol);
                }
                let service =
                    core::str::from_utf8(&frame[2..2 + service_len]).map_err(|_| BlockError::Protocol)?;
                if !valid_service(service) {
                    return Err(BlockError::Protocol);
                }
                zero_tail(frame, 2 + service_len)?;
                Ok(Request::Register { service })
            }
            LIST => {
                zero_tail(frame, 1)?;
                Ok(Request::List)
            }
            PROBE => {
                let name = decode_name(frame, frame[1] as usize)?;
                zero_tail(frame, 2 + name.len())?;
                Ok(Request::Probe { name })
            }
            READ => {
                let name = decode_name(frame, frame[1] as usize)?;
                let count = read_u16(frame, 34);
                if !valid_count(count) {
                    return Err(BlockError::Protocol);
                }
                zero_region(frame, 2 + name.len(), 26)?;
                zero_tail(frame, 36)?;
                Ok(Request::Read {
                    name,
                    block: read_u64(frame, 26),
                    count,
                })
            }
            WRITE => {
                let name = decode_name(frame, frame[1] as usize)?;
                let count = read_u16(frame, 34);
                if !valid_count(count) {
                    return Err(BlockError::Protocol);
                }
                zero_region(frame, 2 + name.len(), 26)?;
                zero_tail(frame, 40)?;
                Ok(Request::Write {
                    name,
                    block: read_u64(frame, 26),
                    count,
                    token: read_u32(frame, 36),
                })
            }
            FLUSH => {
                let name = decode_name(frame, frame[1] as usize)?;
                zero_tail(frame, 2 + name.len())?;
                Ok(Request::Flush { name })
            }
            _ => Err(BlockError::Protocol),
        }
    }
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
                kind,
                state,
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
                frame[13] = kind.to_byte();
                frame[14] = state.to_byte();
                put_padded(&mut frame[15..15 + DESCRIPTION_BYTES], description.as_bytes());
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
                let kind = DeviceKind::from_byte(frame[13]).ok_or(BlockError::Protocol)?;
                let state = DeviceState::from_byte(frame[14]).ok_or(BlockError::Protocol)?;
                let description = description_str(frame)?;
                zero_tail(frame, 59)?;
                Ok(Reply::Probe {
                    block_count: read_u64(frame, 1),
                    block_size,
                    kind,
                    state,
                    description,
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

fn description_str(frame: &[u8; FRAME_BYTES]) -> Result<&str, BlockError> {
    let field = &frame[15..15 + DESCRIPTION_BYTES];
    let text = core::str::from_utf8(field).map_err(|_| BlockError::Protocol)?;
    Ok(text.trim_end_matches(' '))
}

fn padded_field<'a>(raw: &'a [u8]) -> Result<&'a str, BlockError> {
    let text = core::str::from_utf8(raw).map_err(|_| BlockError::Protocol)?;
    Ok(text.trim_end_matches(' '))
}

fn zero_region(frame: &[u8; FRAME_BYTES], from: usize, to: usize) -> Result<(), BlockError> {
    if frame[from..to].iter().any(|byte| *byte != 0) {
        Err(BlockError::Protocol)
    } else {
        Ok(())
    }
}

fn zero_tail(frame: &[u8; FRAME_BYTES], from: usize) -> Result<(), BlockError> {
    zero_region(frame, from, FRAME_BYTES)
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

pub fn encode_device_list(devices: &[DeviceInfo]) -> Result<Vec<u8>, BlockError> {
    let mut blob = Vec::with_capacity(4 + devices.len() * LIST_ENTRY_BYTES);
    blob.extend_from_slice(&(devices.len() as u32).to_le_bytes());
    for device in devices {
        if !valid_device_name(&device.name) || !valid_source(&device.source) {
            return Err(BlockError::Protocol);
        }
        if !valid_block_size(device.block_size) || device.description.len() > DESCRIPTION_BYTES {
            return Err(BlockError::Protocol);
        }
        let mut entry = [0u8; LIST_ENTRY_BYTES];
        put_padded(&mut entry[0..NAME_BYTES], device.name.as_bytes());
        put_padded(&mut entry[NAME_BYTES..2 * NAME_BYTES], device.source.as_bytes());
        put_padded(&mut entry[48..48 + DESCRIPTION_BYTES], device.description.as_bytes());
        entry[92..100].copy_from_slice(&device.block_count.to_le_bytes());
        entry[100..104].copy_from_slice(&device.block_size.to_le_bytes());
        entry[104] = device.kind.to_byte();
        entry[105] = device.state.to_byte();
        blob.extend_from_slice(&entry);
    }
    Ok(blob)
}

pub fn decode_device_list(blob: &[u8]) -> Result<Vec<DeviceInfo>, BlockError> {
    if blob.len() < 4 {
        return Err(BlockError::Protocol);
    }
    let count = u32::from_le_bytes(blob[0..4].try_into().unwrap()) as usize;
    if count > (usize::MAX - 4) / LIST_ENTRY_BYTES || blob.len() != 4 + count * LIST_ENTRY_BYTES {
        return Err(BlockError::Protocol);
    }
    let mut devices = Vec::with_capacity(count);
    for index in 0..count {
        let entry = &blob[4 + index * LIST_ENTRY_BYTES..4 + (index + 1) * LIST_ENTRY_BYTES];
        let name = padded_field(&entry[0..NAME_BYTES])?;
        if !valid_device_name(name) {
            return Err(BlockError::Protocol);
        }
        let source = padded_field(&entry[NAME_BYTES..2 * NAME_BYTES])?;
        if !valid_source(source) {
            return Err(BlockError::Protocol);
        }
        let description = padded_field(&entry[48..48 + DESCRIPTION_BYTES])?;
        let block_size = u32::from_le_bytes(entry[100..104].try_into().unwrap());
        if !valid_block_size(block_size) {
            return Err(BlockError::Protocol);
        }
        let kind = DeviceKind::from_byte(entry[104]).ok_or(BlockError::Protocol)?;
        let state = DeviceState::from_byte(entry[105]).ok_or(BlockError::Protocol)?;
        if entry[106..LIST_ENTRY_BYTES].iter().any(|byte| *byte != 0) {
            return Err(BlockError::Protocol);
        }
        devices.push(DeviceInfo {
            name: String::from(name),
            source: String::from(source),
            description: String::from(description),
            block_count: u64::from_le_bytes(entry[92..100].try_into().unwrap()),
            block_size,
            kind,
            state,
        });
    }
    Ok(devices)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text_of_len(length: usize) -> String {
        (0..length).map(|_| 'k').collect()
    }

    fn printable_text_of_len(length: usize) -> String {
        (0..length).map(|index| (b'a' + (index % 26) as u8) as char).collect()
    }

    fn device_named(name: &str) -> DeviceInfo {
        DeviceInfo {
            name: String::from(name),
            source: String::from("ATA"),
            description: String::from("QEMU HARDDISK"),
            block_count: 102400,
            block_size: 512,
            kind: DeviceKind::Disk,
            state: DeviceState::Clean,
        }
    }

    #[test]
    fn opcode_space_is_pinned_and_replies_are_disjoint() {
        assert_eq!(REGISTER, 0x01);
        assert_eq!(LIST, 0x02);
        assert_eq!(PROBE, 0x03);
        assert_eq!(READ, 0x04);
        assert_eq!(WRITE, 0x05);
        assert_eq!(FLUSH, 0x06);
        assert_eq!(REPLY_ACK, 0x10);
        assert_eq!(REPLY_ERROR, 0x11);
        assert_eq!(REPLY_PROBE, 0x12);
        assert_eq!(REPLY_DATA, 0x13);
        assert_eq!(SERVICE_NAME, "BLOCKD");
        assert_eq!(NAME_BYTES, 24);
        assert_eq!(SERVICE_STRING_BUDGET, 58);
        assert_eq!(DESCRIPTION_BYTES, 44);
        assert_eq!(LIST_ENTRY_BYTES, 112);
        let opcodes = [REGISTER, LIST, PROBE, READ, WRITE, FLUSH];
        for tag in [REPLY_ACK, REPLY_ERROR, REPLY_PROBE, REPLY_DATA] {
            assert!(!opcodes.contains(&tag), "tag {tag:#04x} collides with an opcode");
        }
    }

    #[test]
    fn list_entry_field_offsets_fill_the_record_exactly() {
        assert_eq!(2 * NAME_BYTES + DESCRIPTION_BYTES, 92);
        assert_eq!(92 + 8 + 4 + 1 + 1 + 6, LIST_ENTRY_BYTES);
    }

    #[test]
    fn register_frame_pins_service_offset() {
        let frame = Request::Register { service: "ATA" }.encode().unwrap();
        assert_eq!(frame[0], REGISTER);
        assert_eq!(frame[1], 3);
        assert_eq!(&frame[2..5], b"ATA");
        assert!(frame[5..].iter().all(|byte| *byte == 0));
    }

    #[test]
    fn register_roundtrips_the_full_service_budget() {
        let service = printable_text_of_len(SERVICE_STRING_BUDGET);
        let frame = Request::Register { service: &service }.encode().unwrap();
        assert_eq!(Request::decode(&frame), Ok(Request::Register { service: &service }));
    }

    #[test]
    fn register_rejects_services_beyond_the_budget() {
        let over = printable_text_of_len(SERVICE_STRING_BUDGET + 1);
        assert_eq!(Request::Register { service: &over }.encode(), Err(BlockError::Protocol));
        let mut frame = Request::Register { service: "ATA" }.encode().unwrap();
        frame[1] = (SERVICE_STRING_BUDGET + 1) as u8;
        assert_eq!(Request::decode(&frame), Err(BlockError::Protocol));
    }

    #[test]
    fn register_rejects_empty_and_non_printable_services() {
        assert_eq!(Request::Register { service: "" }.encode(), Err(BlockError::Protocol));
        assert_eq!(
            Request::Register { service: "BAD SERVICE" }.encode(),
            Err(BlockError::Protocol)
        );
        assert_eq!(
            Request::Register { service: "BAD\x00SERVICE" }.encode(),
            Err(BlockError::Protocol)
        );
        let mut frame = Request::Register { service: "ATA" }.encode().unwrap();
        frame[3] = b' ';
        assert_eq!(Request::decode(&frame), Err(BlockError::Protocol));
    }

    #[test]
    fn list_frame_is_the_op_byte_with_a_zero_tail() {
        let frame = Request::List.encode().unwrap();
        assert_eq!(frame[0], LIST);
        assert!(frame[1..].iter().all(|byte| *byte == 0));
        assert_eq!(Request::decode(&frame), Ok(Request::List));
        let mut polluted = Request::List.encode().unwrap();
        polluted[30] = 1;
        assert_eq!(Request::decode(&polluted), Err(BlockError::Protocol));
    }

    #[test]
    fn probe_frame_pins_the_fixed_name_field() {
        let frame = Request::Probe { name: "ata0d1" }.encode().unwrap();
        assert_eq!(frame[0], PROBE);
        assert_eq!(frame[1], 6);
        assert_eq!(&frame[2..8], b"ata0d1");
        assert!(frame[8..].iter().all(|byte| *byte == 0));
    }

    #[test]
    fn read_frame_pins_little_endian_offsets() {
        let frame = Request::Read {
            name: "ata0d0",
            block: 0x0807_0605_0403_0201,
            count: 8,
        }
        .encode()
        .unwrap();
        assert_eq!(frame[0], READ);
        assert_eq!(frame[1], 6);
        assert_eq!(&frame[2..8], b"ata0d0");
        assert_eq!(
            &frame[26..34],
            &[0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08]
        );
        assert_eq!(&frame[34..36], &[0x08, 0x00]);
        assert!(frame[36..].iter().all(|byte| *byte == 0));
    }

    #[test]
    fn write_frame_pins_the_data_token_after_the_read_shape() {
        let frame = Request::Write {
            name: "ata0d1",
            block: 4096,
            count: 128,
            token: 0x00AB_CDEF,
        }
        .encode()
        .unwrap();
        assert_eq!(frame[0], WRITE);
        assert_eq!(frame[1], 6);
        assert_eq!(&frame[2..8], b"ata0d1");
        assert_eq!(
            &frame[26..34],
            &[0x00, 0x10, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]
        );
        assert_eq!(&frame[34..36], &[0x80, 0x00]);
        assert_eq!(&frame[36..40], &[0xEF, 0xCD, 0xAB, 0x00]);
        assert!(frame[40..].iter().all(|byte| *byte == 0));
    }

    #[test]
    fn flush_frame_shares_the_probe_shape() {
        let probe = Request::Probe { name: "ata0d1" }.encode().unwrap();
        let mut flush = Request::Flush { name: "ata0d1" }.encode().unwrap();
        flush[0] = PROBE;
        assert_eq!(flush, probe);
    }

    #[test]
    fn request_roundtrips_all_variants() {
        let requests = [
            Request::Register { service: "ATA" },
            Request::List,
            Request::Probe { name: "a" },
            Request::Probe { name: &text_of_len(NAME_BYTES) },
            Request::Read {
                name: "ata0d1",
                block: u64::MAX,
                count: 1,
            },
            Request::Read {
                name: "ata0d1",
                block: 0,
                count: crate::wire::MAX_BLOCKS_PER_REQUEST as u16,
            },
            Request::Write {
                name: "ata0d0",
                block: 9,
                count: 33,
                token: 0x7FFF_FFFF,
            },
            Request::Flush { name: "disk-7" },
        ];
        for request in requests {
            assert_eq!(Request::decode(&request.encode().unwrap()), Ok(request));
        }
    }

    #[test]
    fn decode_request_rejects_opcodes_outside_the_space() {
        for op in [0u8, 0x07, 0x0F, 0x10, 0x20, 0xFF] {
            let mut frame = Request::List.encode().unwrap();
            frame[0] = op;
            assert_eq!(
                Request::decode(&frame),
                Err(BlockError::Protocol),
                "op byte {op:#04x} accepted"
            );
        }
    }

    #[test]
    fn device_name_validity_is_lowercase_digits_and_inner_dash() {
        assert!(valid_device_name("a"));
        assert!(valid_device_name("0"));
        assert!(valid_device_name("ata0d1"));
        assert!(valid_device_name("ata0d0p3"));
        assert!(valid_device_name("a-1"));
        assert!(valid_device_name(&text_of_len(NAME_BYTES)));
        assert!(!valid_device_name(""));
        assert!(!valid_device_name(&text_of_len(NAME_BYTES + 1)));
        assert!(!valid_device_name("-lead"));
        assert!(!valid_device_name("ATA0"));
        assert!(!valid_device_name("ata_0"));
        assert!(!valid_device_name("ata.0"));
        assert!(!valid_device_name("ata/0"));
        assert!(!valid_device_name("ata 0"));
    }

    #[test]
    fn name_bearing_requests_reject_invalid_names_on_encode_and_decode() {
        for name in ["", "-x", "UPPER", "bad name", &text_of_len(NAME_BYTES + 1)] {
            assert_eq!(
                Request::Probe { name }.encode(),
                Err(BlockError::Protocol),
                "probe encode accepted {name:?}"
            );
            assert_eq!(
                Request::Read { name, block: 0, count: 1 }.encode(),
                Err(BlockError::Protocol),
                "read encode accepted {name:?}"
            );
            assert_eq!(
                Request::Flush { name }.encode(),
                Err(BlockError::Protocol),
                "flush encode accepted {name:?}"
            );
        }
        for length in [0u8, 25, 255] {
            for op in [PROBE, READ, WRITE, FLUSH] {
                let mut frame = Request::Probe { name: "ata0d1" }.encode().unwrap();
                frame[0] = op;
                frame[1] = length;
                assert_eq!(
                    Request::decode(&frame),
                    Err(BlockError::Protocol),
                    "{op:#04x} decode accepted name_len {length}"
                );
            }
        }
    }

    #[test]
    fn request_bounds_reject_count_zero_and_above_maximum() {
        for count in [0u16, 129, u16::MAX] {
            let mut frame = Request::Read {
                name: "ata0d1",
                block: 0,
                count: 1,
            }
            .encode()
            .unwrap();
            frame[34..36].copy_from_slice(&count.to_le_bytes());
            assert_eq!(
                Request::decode(&frame),
                Err(BlockError::Protocol),
                "count {count} accepted"
            );
            assert_eq!(
                Request::Read {
                    name: "ata0d1",
                    block: 0,
                    count,
                }
                .encode(),
                Err(BlockError::Protocol)
            );
        }
    }

    #[test]
    fn decode_rejects_nonzero_name_padding_and_frames_tails() {
        let mut probe = Request::Probe { name: "ata" }.encode().unwrap();
        probe[8] = 0x40;
        assert_eq!(Request::decode(&probe), Err(BlockError::Protocol));
        let mut probe_tail = Request::Probe { name: "ata" }.encode().unwrap();
        probe_tail[26] = 1;
        assert_eq!(Request::decode(&probe_tail), Err(BlockError::Protocol));
        let mut read_pad = Request::Read {
            name: "ata",
            block: 0,
            count: 1,
        }
        .encode()
        .unwrap();
        read_pad[10] = 1;
        assert_eq!(Request::decode(&read_pad), Err(BlockError::Protocol));
        let mut read_tail = Request::Read {
            name: "ata0d1",
            block: 0,
            count: 1,
        }
        .encode()
        .unwrap();
        read_tail[36] = 1;
        assert_eq!(Request::decode(&read_tail), Err(BlockError::Protocol));
        let mut write_tail = Request::Write {
            name: "ata0d1",
            block: 0,
            count: 1,
            token: 7,
        }
        .encode()
        .unwrap();
        write_tail[40] = 1;
        assert_eq!(Request::decode(&write_tail), Err(BlockError::Protocol));
        let mut register_tail = Request::Register { service: "ATA" }.encode().unwrap();
        register_tail[5] = 1;
        assert_eq!(Request::decode(&register_tail), Err(BlockError::Protocol));
    }

    #[test]
    fn name_padding_between_the_field_and_the_geometry_survives_reads() {
        let frame = Request::Read {
            name: "ata0d1",
            block: 7,
            count: 2,
        }
        .encode()
        .unwrap();
        assert!(frame[8..26].iter().all(|byte| *byte == 0));
        assert_eq!(
            Request::decode(&frame),
            Ok(Request::Read {
                name: "ata0d1",
                block: 7,
                count: 2
            })
        );
    }

    #[test]
    fn ack_reply_pins_tag_and_zero_tail() {
        let frame = Reply::Ack.encode().unwrap();
        assert_eq!(frame[0], REPLY_ACK);
        assert!(frame[1..].iter().all(|byte| *byte == 0));
        let mut polluted = Reply::Ack.encode().unwrap();
        polluted[30] = 0x40;
        assert_eq!(Reply::decode(&polluted), Err(BlockError::Protocol));
    }

    #[test]
    fn error_reply_pins_tag_and_error_byte() {
        let frame = Reply::Err(BlockError::NotFound).encode().unwrap();
        assert_eq!(frame[0], REPLY_ERROR);
        assert_eq!(frame[1], 1);
        assert!(frame[2..].iter().all(|byte| *byte == 0));
        let mut polluted = Reply::Err(BlockError::Busy).encode().unwrap();
        polluted[2] = 1;
        assert_eq!(Reply::decode(&polluted), Err(BlockError::Protocol));
    }

    #[test]
    fn probe_reply_pins_field_offsets_including_kind_and_state() {
        let frame = Reply::Probe {
            block_count: 0x0000_0100_0000_0001,
            block_size: 4096,
            kind: DeviceKind::Partition,
            state: DeviceState::Partitioned,
            description: "ata0d1p0 slice",
        }
        .encode()
        .unwrap();
        assert_eq!(frame[0], REPLY_PROBE);
        assert_eq!(
            &frame[1..9],
            &[0x01, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00]
        );
        assert_eq!(&frame[9..13], &[0x00, 0x10, 0x00, 0x00]);
        assert_eq!(frame[13], 1);
        assert_eq!(frame[14], 1);
        assert_eq!(&frame[15..29], b"ata0d1p0 slice");
        assert!(frame[29..59].iter().all(|byte| *byte == b' '));
        assert_eq!(frame[59], 0);
    }

    #[test]
    fn probe_reply_accepts_every_kind_and_state() {
        for kind in [DeviceKind::Disk, DeviceKind::Partition] {
            for state in [DeviceState::Clean, DeviceState::Partitioned, DeviceState::Corrupt] {
                let reply = Reply::Probe {
                    block_count: 8,
                    block_size: 512,
                    kind,
                    state,
                    description: "",
                };
                assert_eq!(Reply::decode(&reply.encode().unwrap()), Ok(reply));
            }
        }
    }

    #[test]
    fn probe_reply_rejects_unknown_kind_and_state_bytes() {
        for byte in [3u8, 128, 255] {
            let mut frame = Reply::Probe {
                block_count: 1,
                block_size: 512,
                kind: DeviceKind::Disk,
                state: DeviceState::Clean,
                description: "",
            }
            .encode()
            .unwrap();
            frame[13] = byte;
            assert_eq!(
                Reply::decode(&frame),
                Err(BlockError::Protocol),
                "kind byte {byte} accepted"
            );
            let mut frame = Reply::Probe {
                block_count: 1,
                block_size: 512,
                kind: DeviceKind::Disk,
                state: DeviceState::Clean,
                description: "",
            }
            .encode()
            .unwrap();
            frame[14] = byte;
            assert_eq!(
                Reply::decode(&frame),
                Err(BlockError::Protocol),
                "state byte {byte} accepted"
            );
        }
        let mut frame = Reply::Probe {
            block_count: 1,
            block_size: 512,
            kind: DeviceKind::Disk,
            state: DeviceState::Clean,
            description: "",
        }
        .encode()
        .unwrap();
        frame[13] = 2;
        assert_eq!(Reply::decode(&frame), Err(BlockError::Protocol), "kind byte 2 accepted");
    }

    #[test]
    fn probe_reply_accepts_the_full_description_width() {
        let description = text_of_len(DESCRIPTION_BYTES);
        let reply = Reply::Probe {
            block_count: 1,
            block_size: 512,
            kind: DeviceKind::Disk,
            state: DeviceState::Clean,
            description: &description,
        };
        assert_eq!(Reply::decode(&reply.encode().unwrap()), Ok(reply));
    }

    #[test]
    fn probe_reply_rejects_an_overlong_description() {
        assert_eq!(
            Reply::Probe {
                block_count: 1,
                block_size: 512,
                kind: DeviceKind::Disk,
                state: DeviceState::Clean,
                description: &text_of_len(DESCRIPTION_BYTES + 1),
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
            kind: DeviceKind::Disk,
            state: DeviceState::Clean,
            description: "ok\x01\x7f!",
        }
        .encode()
        .unwrap();
        assert_eq!(&frame[15..19], b"ok  ");
        assert_eq!(&frame[19..21], b"! ");
    }

    #[test]
    fn probe_reply_decode_rejects_invalid_utf8_description() {
        let mut frame = Reply::Probe {
            block_count: 1,
            block_size: 512,
            kind: DeviceKind::Disk,
            state: DeviceState::Clean,
            description: "ok",
        }
        .encode()
        .unwrap();
        frame[17] = 0xC3;
        frame[18] = 0x28;
        assert_eq!(Reply::decode(&frame), Err(BlockError::Protocol));
    }

    #[test]
    fn probe_reply_rejects_block_size_zero_and_non_power_of_two() {
        for block_size in [0u32, 1000, 3, u32::MAX] {
            assert_eq!(
                Reply::Probe {
                    block_count: 1,
                    block_size,
                    kind: DeviceKind::Disk,
                    state: DeviceState::Clean,
                    description: "d",
                }
                .encode(),
                Err(BlockError::Protocol),
                "block size {block_size} accepted on encode"
            );
            let mut frame = Reply::Probe {
                block_count: 1,
                block_size: 4096,
                kind: DeviceKind::Disk,
                state: DeviceState::Clean,
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
    fn probe_reply_decode_rejects_the_reserved_tail_byte() {
        let mut frame = Reply::Probe {
            block_count: 1,
            block_size: 512,
            kind: DeviceKind::Disk,
            state: DeviceState::Clean,
            description: "",
        }
        .encode()
        .unwrap();
        frame[59] = 1;
        assert_eq!(Reply::decode(&frame), Err(BlockError::Protocol));
    }

    #[test]
    fn data_reply_pins_token_and_rejects_a_dirty_tail() {
        let frame = Reply::Data { token: 0x0102_0304 }.encode().unwrap();
        assert_eq!(frame[0], REPLY_DATA);
        assert_eq!(&frame[1..5], &[0x04, 0x03, 0x02, 0x01]);
        assert!(frame[5..].iter().all(|byte| *byte == 0));
        let mut polluted = Reply::Data { token: 1 }.encode().unwrap();
        polluted[5] = 1;
        assert_eq!(Reply::decode(&polluted), Err(BlockError::Protocol));
    }

    #[test]
    fn reply_all_variants_roundtrip() {
        let replies = [
            Reply::Ack,
            Reply::Err(BlockError::NotFound),
            Reply::Err(BlockError::Unsupported),
            Reply::Probe {
                block_count: 102400,
                block_size: 512,
                kind: DeviceKind::Disk,
                state: DeviceState::Clean,
                description: "QEMU HARDDISK 50MB",
            },
            Reply::Probe {
                block_count: u64::MAX,
                block_size: 4096,
                kind: DeviceKind::Partition,
                state: DeviceState::Corrupt,
                description: &text_of_len(DESCRIPTION_BYTES),
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
        for tag in [0x00u8, 0x0F, 0x14, 0x2F, 0xFF] {
            let mut frame = [0u8; FRAME_BYTES];
            frame[0] = tag;
            assert_eq!(Reply::decode(&frame), Err(BlockError::Protocol), "tag {tag:#04x} accepted");
        }
    }

    #[test]
    fn empty_device_list_is_the_four_byte_zero_count() {
        let blob = encode_device_list(&[]).unwrap();
        assert_eq!(blob, vec![0, 0, 0, 0]);
        assert_eq!(decode_device_list(&blob), Ok(Vec::new()));
    }

    #[test]
    fn device_list_roundtrips_devices() {
        let devices = vec![
            device_named("ata0d0"),
            DeviceInfo {
                name: String::from("ata0d1"),
                source: String::from("ATA"),
                description: String::from(""),
                block_count: u64::MAX,
                block_size: 4096,
                kind: DeviceKind::Partition,
                state: DeviceState::Corrupt,
            },
        ];
        let blob = encode_device_list(&devices).unwrap();
        assert_eq!(blob.len(), 4 + 2 * LIST_ENTRY_BYTES);
        assert_eq!(decode_device_list(&blob), Ok(devices));
    }

    #[test]
    fn device_list_entry_pads_fields_and_pins_offsets() {
        let blob = encode_device_list(&[device_named("ata0d1")]).unwrap();
        let entry = &blob[4..4 + LIST_ENTRY_BYTES];
        assert_eq!(&entry[0..6], b"ata0d1");
        assert!(entry[6..NAME_BYTES].iter().all(|byte| *byte == b' '));
        assert_eq!(&entry[24..27], b"ATA");
        assert!(entry[27..48].iter().all(|byte| *byte == b' '));
        assert_eq!(&entry[48..61], b"QEMU HARDDISK");
        assert!(entry[61..92].iter().all(|byte| *byte == b' '));
        assert_eq!(&entry[92..100], &102400u64.to_le_bytes());
        assert_eq!(&entry[100..104], &512u32.to_le_bytes());
        assert_eq!(entry[104], 0);
        assert_eq!(entry[105], 0);
        assert!(entry[106..].iter().all(|byte| *byte == 0));
    }

    #[test]
    fn device_list_fills_the_field_widths_exactly() {
        let device = DeviceInfo {
            name: text_of_len(NAME_BYTES),
            source: printable_text_of_len(NAME_BYTES),
            description: printable_text_of_len(DESCRIPTION_BYTES),
            block_count: 1,
            block_size: 512,
            kind: DeviceKind::Disk,
            state: DeviceState::Clean,
        };
        let blob = encode_device_list(&[device.clone()]).unwrap();
        assert_eq!(decode_device_list(&blob), Ok(vec![device]));
    }

    #[test]
    fn device_list_encode_rejects_invalid_fields() {
        for name in ["", "-x", "ATA0", &text_of_len(NAME_BYTES + 1)] {
            assert_eq!(
                encode_device_list(&[device_named(name)]),
                Err(BlockError::Protocol),
                "name {name:?} accepted"
            );
        }
        let mut device = device_named("ata0d0");
        device.description = text_of_len(DESCRIPTION_BYTES + 1);
        assert_eq!(encode_device_list(&[device]), Err(BlockError::Protocol));
        for source in ["", "BAD SERVICE", &text_of_len(NAME_BYTES + 1)] {
            let mut device = device_named("ata0d0");
            device.source = String::from(source);
            assert_eq!(encode_device_list(&[device]), Err(BlockError::Protocol));
        }
        let mut device = device_named("ata0d0");
        device.block_size = 1000;
        assert_eq!(encode_device_list(&[device]), Err(BlockError::Protocol));
    }

    #[test]
    fn device_list_decode_rejects_length_mismatch() {
        assert_eq!(decode_device_list(&[]), Err(BlockError::Protocol));
        assert_eq!(decode_device_list(&[0, 0, 0]), Err(BlockError::Protocol));
        assert_eq!(decode_device_list(&[1, 0, 0, 0]), Err(BlockError::Protocol));
        let blob = encode_device_list(&[device_named("ata0d0")]).unwrap();
        assert_eq!(decode_device_list(&blob[..blob.len() - 1]), Err(BlockError::Protocol));
        let mut longer = blob.clone();
        longer.push(0);
        assert_eq!(decode_device_list(&longer), Err(BlockError::Protocol));
    }

    #[test]
    fn device_list_decode_rejects_count_overflow() {
        let mut blob = vec![0u8; 4];
        blob[0..4].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(decode_device_list(&blob), Err(BlockError::Protocol));
    }

    #[test]
    fn device_list_decode_rejects_corrupt_entries() {
        let mut blob = encode_device_list(&[device_named("ata0d0")]).unwrap();
        blob[4] = b'A';
        assert_eq!(decode_device_list(&blob), Err(BlockError::Protocol));
        let mut blob = encode_device_list(&[device_named("ata0d0")]).unwrap();
        blob[4 + 24] = b' ';
        blob[4 + 25] = b' ';
        blob[4 + 26] = b' ';
        assert_eq!(decode_device_list(&blob), Err(BlockError::Protocol));
        let mut blob = encode_device_list(&[device_named("ata0d0")]).unwrap();
        blob[4 + 104] = 9;
        assert_eq!(decode_device_list(&blob), Err(BlockError::Protocol));
        let mut blob = encode_device_list(&[device_named("ata0d0")]).unwrap();
        blob[4 + 105] = 9;
        assert_eq!(decode_device_list(&blob), Err(BlockError::Protocol));
        let mut blob = encode_device_list(&[device_named("ata0d0")]).unwrap();
        blob[4 + 106] = 1;
        assert_eq!(decode_device_list(&blob), Err(BlockError::Protocol));
        let mut blob = encode_device_list(&[device_named("ata0d0")]).unwrap();
        blob[4 + 100..4 + 104].copy_from_slice(&1000u32.to_le_bytes());
        assert_eq!(decode_device_list(&blob), Err(BlockError::Protocol));
        let mut blob = encode_device_list(&[device_named("ata0d0")]).unwrap();
        blob[4 + 48] = 0xC3;
        blob[4 + 49] = 0x28;
        assert_eq!(decode_device_list(&blob), Err(BlockError::Protocol));
    }

    #[test]
    fn device_state_labels_are_stable() {
        assert_eq!(DeviceState::Clean.label(), "clean");
        assert_eq!(DeviceState::Partitioned.label(), "partitioned");
        assert_eq!(DeviceState::Corrupt.label(), "corrupt");
    }

    fn seed_frames() -> Vec<[u8; FRAME_BYTES]> {
        let requests = [
            Request::Register { service: "ATA" },
            Request::List,
            Request::Probe { name: "ata0d1" },
            Request::Read {
                name: "ata0d1",
                block: 7,
                count: 1,
            },
            Request::Write {
                name: "ata0d0",
                block: 0,
                count: 128,
                token: 0x00AB_CDEF,
            },
            Request::Flush { name: "ata0d0" },
        ];
        let replies = [
            Reply::Ack,
            Reply::Err(BlockError::Device),
            Reply::Probe {
                block_count: 102400,
                block_size: 512,
                kind: DeviceKind::Disk,
                state: DeviceState::Clean,
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

    #[test]
    fn decoding_device_list_flips_never_panics() {
        let seed = encode_device_list(&[device_named("ata0d1")]).unwrap();
        for index in 0..seed.len() {
            for bit in 0..8 {
                let mut mutated = seed.clone();
                mutated[index] ^= 1 << bit;
                let _ = decode_device_list(&mutated);
            }
        }
    }
}
