use alloc::format;
use alloc::vec;
use alloc::vec::Vec;

pub const SERVICE_NAME: &str = "IP";
pub const FRAME_BYTES: usize = 60;
pub const IPV4_BYTES: usize = 4;
pub const MAC_BYTES: usize = 6;
pub const MAX_IP_PAYLOAD_BYTES: usize = 1480;
pub const NEIGH_STRIDE_BYTES: usize = 24;
pub const MAX_NEIGH_ENTRIES: usize = 64;
pub const KIND_ALL: u8 = 255;

pub const STATUS: u8 = 0x01;
pub const CONFIG: u8 = 0x02;
pub const NEIGH: u8 = 0x03;
pub const SUBSCRIBE: u8 = 0x04;
pub const SEND_IP: u8 = 0x05;

pub const REPLY_ACK: u8 = 0x10;
pub const REPLY_ERROR: u8 = 0x11;
pub const REPLY_STATUS: u8 = 0x12;
pub const REPLY_NEIGH: u8 = 0x13;

pub const PUSH_DATAGRAM: u8 = 0x20;
pub const PUSH_UNREACHABLE: u8 = 0x21;

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum IpError {
    NotConfigured = 1,
    NoRoute = 2,
    Oversize = 3,
    Congested = 4,
    Protocol = 255,
}

impl IpError {
    pub fn to_byte(self) -> u8 {
        self as u8
    }

    pub fn from_byte(byte: u8) -> IpError {
        match byte {
            1 => IpError::NotConfigured,
            2 => IpError::NoRoute,
            3 => IpError::Oversize,
            4 => IpError::Congested,
            _ => IpError::Protocol,
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum UnreachableReason {
    NoRoute,
    NotConfigured,
    Oversize,
    Congested,
}

impl UnreachableReason {
    pub fn code(self) -> u8 {
        match self {
            UnreachableReason::NoRoute => 0,
            UnreachableReason::NotConfigured => 1,
            UnreachableReason::Oversize => 2,
            UnreachableReason::Congested => 3,
        }
    }

    pub fn from_code(code: u8) -> UnreachableReason {
        match code {
            1 => UnreachableReason::NotConfigured,
            2 => UnreachableReason::Oversize,
            3 => UnreachableReason::Congested,
            _ => UnreachableReason::NoRoute,
        }
    }
}

impl From<IpError> for UnreachableReason {
    fn from(error: IpError) -> UnreachableReason {
        match error {
            IpError::NotConfigured => UnreachableReason::NotConfigured,
            IpError::NoRoute => UnreachableReason::NoRoute,
            IpError::Oversize => UnreachableReason::Oversize,
            IpError::Congested | IpError::Protocol => UnreachableReason::Congested,
        }
    }
}

#[derive(Debug, Copy, Clone, Default, PartialEq, Eq)]
pub struct StatusCounters {
    pub rx_frames: u32,
    pub tx_frames: u32,
    pub rx_dropped: u32,
    pub tx_dropped: u32,
    pub arp_answers: u32,
    pub echo_answers: u32,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct Status {
    pub link: bool,
    pub configured: bool,
    pub address: [u8; IPV4_BYTES],
    pub prefix_len: u8,
    pub gateway: Option<[u8; IPV4_BYTES]>,
    pub mac: [u8; MAC_BYTES],
    pub counters: StatusCounters,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct Neighbour {
    pub address: [u8; IPV4_BYTES],
    pub mac: [u8; MAC_BYTES],
    pub updated_ms: u64,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum Request {
    Status,
    Config {
        address: [u8; IPV4_BYTES],
        prefix_len: u8,
        gateway: Option<[u8; IPV4_BYTES]>,
    },
    Neigh,
    Subscribe {
        icmp_kind: u8,
    },
    SendIp {
        destination: [u8; IPV4_BYTES],
        protocol: u8,
        token: u32,
        length: u16,
    },
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum Reply {
    Ack,
    Err {
        error: IpError,
    },
    Status(Status),
    Neigh {
        token: u32,
        entries: u16,
    },
    PushDatagram {
        source: [u8; IPV4_BYTES],
        protocol: u8,
        token: u32,
        length: u16,
    },
    PushUnreachable {
        destination: [u8; IPV4_BYTES],
        reason: UnreachableReason,
    },
}

fn valid_payload_length(length: u16) -> bool {
    length as usize >= 1 && length as usize <= MAX_IP_PAYLOAD_BYTES
}

fn zero_tail(frame: &[u8; FRAME_BYTES], from: usize) -> Result<(), IpError> {
    if frame[from..].iter().any(|byte| *byte != 0) {
        Err(IpError::Protocol)
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

impl Request {
    pub fn encode(&self) -> Result<[u8; FRAME_BYTES], IpError> {
        let mut frame = [0u8; FRAME_BYTES];
        match self {
            Request::Status => frame[0] = STATUS,
            Request::Config {
                address,
                prefix_len,
                gateway,
            } => {
                if *prefix_len > 32 {
                    return Err(IpError::Protocol);
                }
                frame[0] = CONFIG;
                frame[1..5].copy_from_slice(address);
                frame[5] = *prefix_len;
                match gateway {
                    None => frame[6] = 0,
                    Some(gateway) => {
                        frame[6] = 1;
                        frame[7..11].copy_from_slice(gateway);
                    }
                }
            }
            Request::Neigh => frame[0] = NEIGH,
            Request::Subscribe { icmp_kind } => {
                frame[0] = SUBSCRIBE;
                frame[1] = *icmp_kind;
            }
            Request::SendIp {
                destination,
                protocol,
                token,
                length,
            } => {
                if !valid_payload_length(*length) {
                    return Err(IpError::Protocol);
                }
                frame[0] = SEND_IP;
                frame[1..5].copy_from_slice(destination);
                frame[5] = *protocol;
                frame[6..10].copy_from_slice(&token.to_le_bytes());
                frame[10..12].copy_from_slice(&length.to_le_bytes());
            }
        }
        Ok(frame)
    }

    pub fn decode(frame: &[u8; FRAME_BYTES]) -> Result<Request, IpError> {
        match frame[0] {
            STATUS => {
                zero_tail(frame, 1)?;
                Ok(Request::Status)
            }
            CONFIG => {
                zero_tail(frame, 11)?;
                if frame[5] > 32 {
                    return Err(IpError::Protocol);
                }
                let address = frame[1..5].try_into().unwrap();
                let gateway = match frame[6] {
                    0 => None,
                    1 => Some(frame[7..11].try_into().unwrap()),
                    _ => return Err(IpError::Protocol),
                };
                Ok(Request::Config {
                    address,
                    prefix_len: frame[5],
                    gateway,
                })
            }
            NEIGH => {
                zero_tail(frame, 1)?;
                Ok(Request::Neigh)
            }
            SUBSCRIBE => {
                zero_tail(frame, 2)?;
                Ok(Request::Subscribe {
                    icmp_kind: frame[1],
                })
            }
            SEND_IP => {
                zero_tail(frame, 12)?;
                let length = read_u16(frame, 10);
                if !valid_payload_length(length) {
                    return Err(IpError::Protocol);
                }
                Ok(Request::SendIp {
                    destination: frame[1..5].try_into().unwrap(),
                    protocol: frame[5],
                    token: read_u32(frame, 6),
                    length,
                })
            }
            _ => Err(IpError::Protocol),
        }
    }
}

impl Reply {
    pub fn encode(&self) -> Result<[u8; FRAME_BYTES], IpError> {
        let mut frame = [0u8; FRAME_BYTES];
        match self {
            Reply::Ack => frame[0] = REPLY_ACK,
            Reply::Err { error } => {
                frame[0] = REPLY_ERROR;
                frame[1] = error.to_byte();
            }
            Reply::Status(status) => {
                frame[0] = REPLY_STATUS;
                frame[1] = u8::from(status.link);
                frame[2] = u8::from(status.configured);
                frame[3..7].copy_from_slice(&status.address);
                frame[7] = status.prefix_len;
                match status.gateway {
                    None => frame[8] = 0,
                    Some(gateway) => {
                        frame[8] = 1;
                        frame[9..13].copy_from_slice(&gateway);
                    }
                }
                frame[13..19].copy_from_slice(&status.mac);
                frame[19..23].copy_from_slice(&status.counters.rx_frames.to_le_bytes());
                frame[23..27].copy_from_slice(&status.counters.tx_frames.to_le_bytes());
                frame[27..31].copy_from_slice(&status.counters.rx_dropped.to_le_bytes());
                frame[31..35].copy_from_slice(&status.counters.tx_dropped.to_le_bytes());
                frame[35..39].copy_from_slice(&status.counters.arp_answers.to_le_bytes());
                frame[39..43].copy_from_slice(&status.counters.echo_answers.to_le_bytes());
            }
            Reply::Neigh { token, entries } => {
                if *entries as usize > MAX_NEIGH_ENTRIES {
                    return Err(IpError::Protocol);
                }
                frame[0] = REPLY_NEIGH;
                frame[1..5].copy_from_slice(&token.to_le_bytes());
                frame[5..7].copy_from_slice(&entries.to_le_bytes());
            }
            Reply::PushDatagram {
                source,
                protocol,
                token,
                length,
            } => {
                if !valid_payload_length(*length) {
                    return Err(IpError::Protocol);
                }
                frame[0] = PUSH_DATAGRAM;
                frame[1..5].copy_from_slice(source);
                frame[5] = *protocol;
                frame[6..10].copy_from_slice(&token.to_le_bytes());
                frame[10..12].copy_from_slice(&length.to_le_bytes());
            }
            Reply::PushUnreachable {
                destination,
                reason,
            } => {
                frame[0] = PUSH_UNREACHABLE;
                frame[1..5].copy_from_slice(destination);
                frame[5] = reason.code();
            }
        }
        Ok(frame)
    }

    pub fn decode(frame: &[u8; FRAME_BYTES]) -> Result<Reply, IpError> {
        match frame[0] {
            REPLY_ACK => {
                zero_tail(frame, 1)?;
                Ok(Reply::Ack)
            }
            REPLY_ERROR => {
                zero_tail(frame, 2)?;
                Ok(Reply::Err {
                    error: IpError::from_byte(frame[1]),
                })
            }
            REPLY_STATUS => {
                zero_tail(frame, 43)?;
                if frame[8] > 1 {
                    return Err(IpError::Protocol);
                }
                Ok(Reply::Status(Status {
                    link: decode_flag(frame[1])?,
                    configured: decode_flag(frame[2])?,
                    address: frame[3..7].try_into().unwrap(),
                    prefix_len: frame[7],
                    gateway: if frame[8] == 1 {
                        Some(frame[9..13].try_into().unwrap())
                    } else {
                        None
                    },
                    mac: frame[13..19].try_into().unwrap(),
                    counters: StatusCounters {
                        rx_frames: read_u32(frame, 19),
                        tx_frames: read_u32(frame, 23),
                        rx_dropped: read_u32(frame, 27),
                        tx_dropped: read_u32(frame, 31),
                        arp_answers: read_u32(frame, 35),
                        echo_answers: read_u32(frame, 39),
                    },
                }))
            }
            REPLY_NEIGH => {
                zero_tail(frame, 7)?;
                let entries = read_u16(frame, 5);
                if entries as usize > MAX_NEIGH_ENTRIES {
                    return Err(IpError::Protocol);
                }
                Ok(Reply::Neigh {
                    token: read_u32(frame, 1),
                    entries,
                })
            }
            PUSH_DATAGRAM => {
                zero_tail(frame, 12)?;
                let length = read_u16(frame, 10);
                if !valid_payload_length(length) {
                    return Err(IpError::Protocol);
                }
                Ok(Reply::PushDatagram {
                    source: frame[1..5].try_into().unwrap(),
                    protocol: frame[5],
                    token: read_u32(frame, 6),
                    length,
                })
            }
            PUSH_UNREACHABLE => {
                zero_tail(frame, 6)?;
                Ok(Reply::PushUnreachable {
                    destination: frame[1..5].try_into().unwrap(),
                    reason: UnreachableReason::from_code(frame[5]),
                })
            }
            _ => Err(IpError::Protocol),
        }
    }
}

fn decode_flag(byte: u8) -> Result<bool, IpError> {
    match byte {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(IpError::Protocol),
    }
}

pub fn encode_neighbours_table(neighbours: &[Neighbour]) -> Vec<u8> {
    let mut blob = vec![0u8; neighbours.len() * NEIGH_STRIDE_BYTES];
    for (index, neighbour) in neighbours.iter().enumerate() {
        let at = index * NEIGH_STRIDE_BYTES;
        blob[at..at + IPV4_BYTES].copy_from_slice(&neighbour.address);
        blob[at + IPV4_BYTES..at + IPV4_BYTES + MAC_BYTES].copy_from_slice(&neighbour.mac);
        blob[at + 10..at + 18].copy_from_slice(&neighbour.updated_ms.to_le_bytes());
    }
    blob
}

pub fn decode_neighbours_table(blob: &[u8]) -> Result<Vec<Neighbour>, IpError> {
    if !blob.len().is_multiple_of(NEIGH_STRIDE_BYTES) {
        return Err(IpError::Protocol);
    }
    let mut neighbours = Vec::with_capacity(blob.len() / NEIGH_STRIDE_BYTES);
    for chunk in blob.chunks_exact(NEIGH_STRIDE_BYTES) {
        if chunk[18..].iter().any(|byte| *byte != 0) {
            return Err(IpError::Protocol);
        }
        neighbours.push(Neighbour {
            address: chunk[0..IPV4_BYTES].try_into().unwrap(),
            mac: chunk[IPV4_BYTES..IPV4_BYTES + MAC_BYTES]
                .try_into()
                .unwrap(),
            updated_ms: u64::from_le_bytes(chunk[10..18].try_into().unwrap()),
        });
    }
    Ok(neighbours)
}

pub fn format_address(address: [u8; IPV4_BYTES]) -> alloc::string::String {
    format!(
        "{}.{}.{}.{}",
        address[0], address[1], address[2], address[3]
    )
}

pub fn parse_address(text: &str) -> Option<[u8; IPV4_BYTES]> {
    let mut octets = [0u8; IPV4_BYTES];
    for (index, part) in text.split('.').enumerate() {
        if index >= IPV4_BYTES || part.is_empty() || part.len() > 3 {
            return None;
        }
        if !part.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        let value: u16 = part.parse().ok()?;
        if value > 255 {
            return None;
        }
        octets[index] = value as u8;
    }
    if text.split('.').count() != IPV4_BYTES {
        return None;
    }
    Some(octets)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ADDR: [u8; IPV4_BYTES] = [10, 0, 2, 15];
    const GW: [u8; IPV4_BYTES] = [10, 0, 2, 2];
    const MAC: [u8; MAC_BYTES] = [0x02, 0x52, 0x4e, 0x45, 0x32, 0x07];

    fn status_sample() -> Status {
        Status {
            link: true,
            configured: true,
            address: ADDR,
            prefix_len: 24,
            gateway: Some(GW),
            mac: MAC,
            counters: StatusCounters {
                rx_frames: 11,
                tx_frames: 22,
                rx_dropped: 33,
                tx_dropped: 44,
                arp_answers: 55,
                echo_answers: 66,
            },
        }
    }

    #[test]
    fn tag_constants_are_pinned_and_disjoint() {
        assert_eq!(STATUS, 0x01);
        assert_eq!(CONFIG, 0x02);
        assert_eq!(NEIGH, 0x03);
        assert_eq!(SUBSCRIBE, 0x04);
        assert_eq!(SEND_IP, 0x05);
        assert_eq!(REPLY_ACK, 0x10);
        assert_eq!(REPLY_ERROR, 0x11);
        assert_eq!(REPLY_STATUS, 0x12);
        assert_eq!(REPLY_NEIGH, 0x13);
        assert_eq!(PUSH_DATAGRAM, 0x20);
        assert_eq!(PUSH_UNREACHABLE, 0x21);
        assert_eq!(FRAME_BYTES, 60);
        assert_eq!(MAX_IP_PAYLOAD_BYTES, 1480);
        assert_eq!(NEIGH_STRIDE_BYTES, 24);
        assert_eq!(KIND_ALL, 255);
        let opcodes = [STATUS, CONFIG, NEIGH, SUBSCRIBE, SEND_IP];
        let tags = [
            REPLY_ACK,
            REPLY_ERROR,
            REPLY_STATUS,
            REPLY_NEIGH,
            PUSH_DATAGRAM,
            PUSH_UNREACHABLE,
        ];
        for tag in tags {
            assert!(
                !opcodes.contains(&tag),
                "tag {tag:#04x} collides with opcode"
            );
        }
    }

    #[test]
    fn error_codes_are_total_and_pinned() {
        assert_eq!(IpError::NotConfigured.to_byte(), 1);
        assert_eq!(IpError::NoRoute.to_byte(), 2);
        assert_eq!(IpError::Oversize.to_byte(), 3);
        assert_eq!(IpError::Congested.to_byte(), 4);
        assert_eq!(IpError::Protocol.to_byte(), 255);
        assert_eq!(IpError::from_byte(0), IpError::Protocol);
        assert_eq!(IpError::from_byte(99), IpError::Protocol);
        for code in 1..=4u8 {
            let error = IpError::from_byte(code);
            assert_eq!(error.to_byte(), code);
        }
    }

    #[test]
    fn unreachable_reason_mapping_is_total_and_lossless() {
        assert_eq!(UnreachableReason::NoRoute.code(), 0);
        assert_eq!(UnreachableReason::from_code(9), UnreachableReason::NoRoute);
        for code in 0..=3u8 {
            assert_eq!(UnreachableReason::from_code(code).code(), code);
        }
        assert_eq!(
            UnreachableReason::from(IpError::NotConfigured),
            UnreachableReason::NotConfigured
        );
        assert_eq!(
            UnreachableReason::from(IpError::NoRoute),
            UnreachableReason::NoRoute
        );
        assert_eq!(
            UnreachableReason::from(IpError::Oversize),
            UnreachableReason::Oversize
        );
        assert_eq!(
            UnreachableReason::from(IpError::Congested),
            UnreachableReason::Congested
        );
    }

    #[test]
    fn request_frames_round_trip() {
        let cases = [
            Request::Status,
            Request::Config {
                address: ADDR,
                prefix_len: 24,
                gateway: Some(GW),
            },
            Request::Config {
                address: ADDR,
                prefix_len: 0,
                gateway: None,
            },
            Request::Config {
                address: [0, 0, 0, 0],
                prefix_len: 32,
                gateway: Some([255, 255, 255, 255]),
            },
            Request::Neigh,
            Request::Subscribe { icmp_kind: 0 },
            Request::Subscribe { icmp_kind: 255 },
            Request::SendIp {
                destination: GW,
                protocol: 1,
                token: 0x00ab_cdef,
                length: 64,
            },
        ];
        for request in cases {
            let frame = request.encode().expect("request encodes");
            assert_eq!(Request::decode(&frame), Ok(request));
        }
    }

    #[test]
    fn config_frame_pins_field_offsets() {
        let frame = Request::Config {
            address: ADDR,
            prefix_len: 24,
            gateway: Some(GW),
        }
        .encode()
        .unwrap();
        assert_eq!(frame[0], CONFIG);
        assert_eq!(&frame[1..5], &ADDR);
        assert_eq!(frame[5], 24);
        assert_eq!(frame[6], 1);
        assert_eq!(&frame[7..11], &GW);
        assert!(frame[11..].iter().all(|byte| *byte == 0));
    }

    #[test]
    fn send_ip_frame_pins_field_offsets() {
        let frame = Request::SendIp {
            destination: GW,
            protocol: 6,
            token: 0x1122_3344,
            length: 1480,
        }
        .encode()
        .unwrap();
        assert_eq!(frame[0], SEND_IP);
        assert_eq!(&frame[1..5], &GW);
        assert_eq!(frame[5], 6);
        assert_eq!(&frame[6..10], &[0x44, 0x33, 0x22, 0x11]);
        assert_eq!(&frame[10..12], &[0xc8, 0x05]);
        assert!(frame[12..].iter().all(|byte| *byte == 0));
    }

    #[test]
    fn request_decode_refuses_dirty_tails_unknown_opcodes_and_ranges() {
        let mut frame = Request::Status.encode().unwrap();
        frame[1] = 1;
        assert_eq!(Request::decode(&frame), Err(IpError::Protocol));
        let mut frame = Request::Subscribe { icmp_kind: 3 }.encode().unwrap();
        frame[2] = 1;
        assert_eq!(Request::decode(&frame), Err(IpError::Protocol));
        for opcode in [0u8, 6, 0x0f, 0x10, 0xff] {
            let mut frame = [0u8; FRAME_BYTES];
            frame[0] = opcode;
            assert_eq!(Request::decode(&frame), Err(IpError::Protocol));
        }
        let mut frame = Request::Config {
            address: ADDR,
            prefix_len: 24,
            gateway: None,
        }
        .encode()
        .unwrap();
        frame[5] = 33;
        assert_eq!(Request::decode(&frame), Err(IpError::Protocol));
        frame[5] = 24;
        frame[6] = 2;
        assert_eq!(Request::decode(&frame), Err(IpError::Protocol));
        let mut frame = Request::SendIp {
            destination: GW,
            protocol: 1,
            token: 1,
            length: 60,
        }
        .encode()
        .unwrap();
        for length in [0u16, 1481, u16::MAX] {
            let mut dirty = frame;
            dirty[10..12].copy_from_slice(&length.to_le_bytes());
            assert_eq!(Request::decode(&dirty), Err(IpError::Protocol));
        }
        frame[12] = 1;
        assert_eq!(Request::decode(&frame), Err(IpError::Protocol));
    }

    #[test]
    fn status_reply_pins_the_whole_frame() {
        let frame = Reply::Status(status_sample()).encode().unwrap();
        assert_eq!(frame[0], REPLY_STATUS);
        assert_eq!(frame[1], 1);
        assert_eq!(frame[2], 1);
        assert_eq!(&frame[3..7], &ADDR);
        assert_eq!(frame[7], 24);
        assert_eq!(frame[8], 1);
        assert_eq!(&frame[9..13], &GW);
        assert_eq!(&frame[13..19], &MAC);
        assert_eq!(&frame[19..23], &11u32.to_le_bytes());
        assert_eq!(&frame[39..43], &66u32.to_le_bytes());
        assert!(frame[43..].iter().all(|byte| *byte == 0));
        assert_eq!(Reply::decode(&frame), Ok(Reply::Status(status_sample())));
    }

    #[test]
    fn status_reply_round_trips_the_unconfigured_shape() {
        let status = Status {
            link: true,
            configured: false,
            address: [0, 0, 0, 0],
            prefix_len: 0,
            gateway: None,
            mac: MAC,
            counters: StatusCounters::default(),
        };
        let frame = Reply::Status(status).encode().unwrap();
        assert_eq!(Reply::decode(&frame), Ok(Reply::Status(status)));
    }

    #[test]
    fn ack_and_error_replies_round_trip() {
        for reply in [
            Reply::Ack,
            Reply::Err {
                error: IpError::NotConfigured,
            },
            Reply::Err {
                error: IpError::Protocol,
            },
        ] {
            let frame = reply.encode().unwrap();
            assert_eq!(Reply::decode(&frame), Ok(reply));
        }
    }

    #[test]
    fn neigh_reply_pins_token_and_entries() {
        let frame = Reply::Neigh {
            token: 0xdead_beef,
            entries: 64,
        }
        .encode()
        .unwrap();
        assert_eq!(frame[0], REPLY_NEIGH);
        assert_eq!(&frame[1..5], &[0xef, 0xbe, 0xad, 0xde]);
        assert_eq!(&frame[5..7], &[64, 0]);
        assert!(frame[7..].iter().all(|byte| *byte == 0));
        assert_eq!(
            Reply::decode(&frame),
            Ok(Reply::Neigh {
                token: 0xdead_beef,
                entries: 64
            })
        );
        assert_eq!(
            Reply::Neigh {
                token: 1,
                entries: 65
            }
            .encode(),
            Err(IpError::Protocol)
        );
    }

    #[test]
    fn push_datagram_pins_offsets_and_bounds() {
        let frame = Reply::PushDatagram {
            source: GW,
            protocol: 1,
            token: 0x0a0b_0c0d,
            length: 84,
        }
        .encode()
        .unwrap();
        assert_eq!(frame[0], PUSH_DATAGRAM);
        assert_eq!(&frame[1..5], &GW);
        assert_eq!(frame[5], 1);
        assert_eq!(&frame[6..10], &[0x0d, 0x0c, 0x0b, 0x0a]);
        assert_eq!(&frame[10..12], &[84, 0]);
        assert!(frame[12..].iter().all(|byte| *byte == 0));
        assert_eq!(
            Reply::decode(&frame),
            Ok(Reply::PushDatagram {
                source: GW,
                protocol: 1,
                token: 0x0a0b_0c0d,
                length: 84
            })
        );
        for length in [0u16, 1481] {
            assert_eq!(
                Reply::PushDatagram {
                    source: GW,
                    protocol: 1,
                    token: 1,
                    length
                }
                .encode(),
                Err(IpError::Protocol)
            );
        }
    }

    #[test]
    fn push_unreachable_round_trips_reasons() {
        for reason in [
            UnreachableReason::NoRoute,
            UnreachableReason::NotConfigured,
            UnreachableReason::Oversize,
            UnreachableReason::Congested,
        ] {
            let reply = Reply::PushUnreachable {
                destination: GW,
                reason,
            };
            let frame = reply.encode().unwrap();
            assert_eq!(frame[0], PUSH_UNREACHABLE);
            assert_eq!(frame[5], reason.code());
            assert_eq!(Reply::decode(&frame), Ok(reply));
        }
    }

    #[test]
    fn reply_decode_refuses_dirty_tails_and_unknown_tags() {
        let seeds = [
            Reply::Ack.encode().unwrap(),
            Reply::Err {
                error: IpError::NoRoute,
            }
            .encode()
            .unwrap(),
            Reply::Status(status_sample()).encode().unwrap(),
            Reply::Neigh {
                token: 2,
                entries: 3,
            }
            .encode()
            .unwrap(),
            Reply::PushDatagram {
                source: ADDR,
                protocol: 1,
                token: 4,
                length: 60,
            }
            .encode()
            .unwrap(),
            Reply::PushUnreachable {
                destination: ADDR,
                reason: UnreachableReason::NoRoute,
            }
            .encode()
            .unwrap(),
        ];
        let dirty_bytes = [1, 2, 43, 7, 12, 6];
        for (seed, dirty_at) in seeds.iter().zip(dirty_bytes) {
            let mut dirty = *seed;
            dirty[dirty_at] = if dirty[dirty_at] == 1 { 2 } else { 1 };
            assert_eq!(Reply::decode(&dirty), Err(IpError::Protocol));
        }
        for tag in [0u8, 0x0f, 0x14, 0x1f, 0xff] {
            let mut frame = [0u8; FRAME_BYTES];
            frame[0] = tag;
            assert_eq!(Reply::decode(&frame), Err(IpError::Protocol));
        }
    }

    #[test]
    fn neighbours_table_round_trips() {
        let neighbours = [
            Neighbour {
                address: GW,
                mac: MAC,
                updated_ms: 0,
            },
            Neighbour {
                address: ADDR,
                mac: [0xff; MAC_BYTES],
                updated_ms: u64::MAX,
            },
        ];
        let blob = encode_neighbours_table(&neighbours);
        assert_eq!(blob.len(), 2 * NEIGH_STRIDE_BYTES);
        assert_eq!(
            decode_neighbours_table(&blob).unwrap().as_slice(),
            &neighbours[..]
        );
        assert!(encode_neighbours_table(&[]).is_empty());
        assert!(decode_neighbours_table(&[]).unwrap().is_empty());
    }

    #[test]
    fn neighbours_table_decode_refuses_garbage() {
        assert_eq!(
            decode_neighbours_table(&[0u8; NEIGH_STRIDE_BYTES - 1]),
            Err(IpError::Protocol)
        );
        let mut blob = encode_neighbours_table(&[Neighbour {
            address: GW,
            mac: MAC,
            updated_ms: 5,
        }]);
        blob[18] = 1;
        assert_eq!(decode_neighbours_table(&blob), Err(IpError::Protocol));
    }

    #[test]
    fn address_format_and_parse_round_trip() {
        assert_eq!(format_address([192, 168, 7, 5]), "192.168.7.5");
        assert_eq!(parse_address("192.168.7.5"), Some([192, 168, 7, 5]));
        assert_eq!(parse_address("0.0.0.0"), Some([0, 0, 0, 0]));
        assert_eq!(parse_address("255.255.255.255"), Some([255, 255, 255, 255]));
        for broken in [
            "1.2.3",
            "1.2.3.4.5",
            "256.0.0.1",
            "1..2.3",
            "a.b.c.d",
            "1.2.3.-4",
            "1.2.3.0444",
            "",
        ] {
            assert_eq!(parse_address(broken), None, "{broken} accepted");
        }
    }

    #[test]
    fn decode_never_panics_on_dirty_frames() {
        let seeds = [
            Request::SendIp {
                destination: ADDR,
                protocol: 1,
                token: 7,
                length: 60,
            }
            .encode()
            .unwrap(),
            Reply::Status(status_sample()).encode().unwrap(),
            Reply::PushDatagram {
                source: ADDR,
                protocol: 1,
                token: 9,
                length: 1,
            }
            .encode()
            .unwrap(),
        ];
        for seed in seeds {
            for byte_index in 0..43usize {
                for bit in 0..8 {
                    let mut mutated = seed;
                    mutated[byte_index] ^= 1 << bit;
                    let _ = Request::decode(&mutated);
                    let _ = Reply::decode(&mutated);
                }
            }
        }
    }
}
