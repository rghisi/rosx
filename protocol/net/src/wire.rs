pub const SERVICE_NAME: &str = "NE2K";
pub const FRAME_BYTES: usize = 60;
pub const MAC_BYTES: usize = 6;
pub const MIN_ETHERNET_BYTES: usize = 60;
pub const MAX_ETHERNET_BYTES: usize = 1514;

pub const MAC: u8 = 0x01;
pub const SEND: u8 = 0x02;
pub const POLL: u8 = 0x03;
pub const STATS: u8 = 0x04;

pub const REPLY_ACK: u8 = 0x10;
pub const REPLY_ERROR: u8 = 0x11;
pub const REPLY_MAC: u8 = 0x12;
pub const REPLY_DATA: u8 = 0x13;
pub const REPLY_NO_FRAME: u8 = 0x14;
pub const REPLY_STATS: u8 = 0x15;

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[repr(u8)]
pub enum NetError {
    NoDevice = 1,
    Busy = 2,
    Oversize = 3,
    Undersize = 4,
    Device = 5,
    Protocol = 255,
}

impl NetError {
    pub fn to_byte(self) -> u8 {
        self as u8
    }

    pub fn from_byte(byte: u8) -> NetError {
        match byte {
            1 => NetError::NoDevice,
            2 => NetError::Busy,
            3 => NetError::Oversize,
            4 => NetError::Undersize,
            5 => NetError::Device,
            _ => NetError::Protocol,
        }
    }
}

fn valid_frame_length(length: u16) -> bool {
    length as usize >= MIN_ETHERNET_BYTES && length as usize <= MAX_ETHERNET_BYTES
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum Request {
    Mac,
    Send { token: u32, length: u16 },
    Poll,
    Stats,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct Stats {
    pub tx_frames: u32,
    pub tx_errors: u32,
    pub rx_frames: u32,
    pub rx_errors: u32,
    pub rx_overruns: u32,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum Reply {
    Ack,
    Err { error: NetError },
    Mac([u8; MAC_BYTES]),
    Data { token: u32, length: u16 },
    NoFrame,
    Stats(Stats),
}

impl Request {
    pub fn encode(&self) -> Result<[u8; FRAME_BYTES], NetError> {
        let mut frame = [0u8; FRAME_BYTES];
        match self {
            Request::Mac => frame[0] = MAC,
            Request::Send { token, length } => {
                if !valid_frame_length(*length) {
                    return Err(if *length as usize > MAX_ETHERNET_BYTES {
                        NetError::Oversize
                    } else {
                        NetError::Undersize
                    });
                }
                frame[0] = SEND;
                frame[1..5].copy_from_slice(&token.to_le_bytes());
                frame[5..7].copy_from_slice(&length.to_le_bytes());
            }
            Request::Poll => frame[0] = POLL,
            Request::Stats => frame[0] = STATS,
        }
        Ok(frame)
    }

    pub fn decode(frame: &[u8; FRAME_BYTES]) -> Result<Request, NetError> {
        match frame[0] {
            MAC => {
                zero_tail(frame, 1)?;
                Ok(Request::Mac)
            }
            SEND => {
                let token = read_u32(frame, 1);
                let length = read_u16(frame, 5);
                zero_tail(frame, 7)?;
                if !valid_frame_length(length) {
                    return Err(NetError::Protocol);
                }
                Ok(Request::Send { token, length })
            }
            POLL => {
                zero_tail(frame, 1)?;
                Ok(Request::Poll)
            }
            STATS => {
                zero_tail(frame, 1)?;
                Ok(Request::Stats)
            }
            _ => Err(NetError::Protocol),
        }
    }
}

impl Reply {
    pub fn encode(&self) -> Result<[u8; FRAME_BYTES], NetError> {
        let mut frame = [0u8; FRAME_BYTES];
        match self {
            Reply::Ack => frame[0] = REPLY_ACK,
            Reply::Err { error } => {
                frame[0] = REPLY_ERROR;
                frame[1] = error.to_byte();
            }
            Reply::Mac(address) => {
                frame[0] = REPLY_MAC;
                frame[1..1 + MAC_BYTES].copy_from_slice(address);
            }
            Reply::Data { token, length } => {
                if !valid_frame_length(*length) {
                    return Err(NetError::Protocol);
                }
                frame[0] = REPLY_DATA;
                frame[1..5].copy_from_slice(&token.to_le_bytes());
                frame[5..7].copy_from_slice(&length.to_le_bytes());
            }
            Reply::NoFrame => frame[0] = REPLY_NO_FRAME,
            Reply::Stats(stats) => {
                frame[0] = REPLY_STATS;
                frame[1..5].copy_from_slice(&stats.tx_frames.to_le_bytes());
                frame[5..9].copy_from_slice(&stats.tx_errors.to_le_bytes());
                frame[9..13].copy_from_slice(&stats.rx_frames.to_le_bytes());
                frame[13..17].copy_from_slice(&stats.rx_errors.to_le_bytes());
                frame[17..21].copy_from_slice(&stats.rx_overruns.to_le_bytes());
            }
        }
        Ok(frame)
    }

    pub fn decode(frame: &[u8; FRAME_BYTES]) -> Result<Reply, NetError> {
        match frame[0] {
            REPLY_ACK => {
                zero_tail(frame, 1)?;
                Ok(Reply::Ack)
            }
            REPLY_ERROR => {
                zero_tail(frame, 2)?;
                Ok(Reply::Err {
                    error: NetError::from_byte(frame[1]),
                })
            }
            REPLY_MAC => {
                zero_tail(frame, 1 + MAC_BYTES)?;
                let mut address = [0u8; MAC_BYTES];
                address.copy_from_slice(&frame[1..1 + MAC_BYTES]);
                Ok(Reply::Mac(address))
            }
            REPLY_DATA => {
                zero_tail(frame, 7)?;
                let length = read_u16(frame, 5);
                if !valid_frame_length(length) {
                    return Err(NetError::Protocol);
                }
                Ok(Reply::Data {
                    token: read_u32(frame, 1),
                    length,
                })
            }
            REPLY_NO_FRAME => {
                zero_tail(frame, 1)?;
                Ok(Reply::NoFrame)
            }
            REPLY_STATS => {
                zero_tail(frame, 21)?;
                Ok(Reply::Stats(Stats {
                    tx_frames: read_u32(frame, 1),
                    tx_errors: read_u32(frame, 5),
                    rx_frames: read_u32(frame, 9),
                    rx_errors: read_u32(frame, 13),
                    rx_overruns: read_u32(frame, 17),
                }))
            }
            _ => Err(NetError::Protocol),
        }
    }
}

fn zero_tail(frame: &[u8; FRAME_BYTES], from: usize) -> Result<(), NetError> {
    if frame[from..].iter().any(|byte| *byte != 0) {
        Err(NetError::Protocol)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tag_constants_are_pinned_and_disjoint() {
        assert_eq!(MAC, 0x01);
        assert_eq!(SEND, 0x02);
        assert_eq!(POLL, 0x03);
        assert_eq!(STATS, 0x04);
        assert_eq!(REPLY_ACK, 0x10);
        assert_eq!(REPLY_ERROR, 0x11);
        assert_eq!(REPLY_MAC, 0x12);
        assert_eq!(REPLY_DATA, 0x13);
        assert_eq!(REPLY_NO_FRAME, 0x14);
        assert_eq!(REPLY_STATS, 0x15);
        let opcodes = [MAC, SEND, POLL, STATS];
        let tags = [
            REPLY_ACK,
            REPLY_ERROR,
            REPLY_MAC,
            REPLY_DATA,
            REPLY_NO_FRAME,
            REPLY_STATS,
        ];
        for tag in tags {
            assert!(!opcodes.contains(&tag), "tag {tag:#04x} collides with opcode");
        }
    }

    #[test]
    fn ethernet_bounds_match_the_frame_sizes() {
        assert_eq!(MIN_ETHERNET_BYTES, 60);
        assert_eq!(MAX_ETHERNET_BYTES, 1514);
        assert!(valid_frame_length(60));
        assert!(valid_frame_length(1514));
        assert!(!valid_frame_length(0));
        assert!(!valid_frame_length(59));
        assert!(!valid_frame_length(1515));
    }

    #[test]
    fn query_request_frames_are_tag_only() {
        for request in [Request::Mac, Request::Poll, Request::Stats] {
            let frame = request.encode().unwrap();
            assert_eq!(frame[0], match request {
                Request::Mac => MAC,
                Request::Poll => POLL,
                _ => STATS,
            });
            assert!(frame[1..].iter().all(|byte| *byte == 0));
            assert_eq!(Request::decode(&frame), Ok(request));
        }
    }

    #[test]
    fn send_frame_pins_token_and_length_offsets() {
        let frame = Request::Send {
            token: 0x0A0B_0C0D,
            length: 1514,
        }
        .encode()
        .unwrap();
        assert_eq!(frame[0], SEND);
        assert_eq!(&frame[1..5], &[0x0D, 0x0C, 0x0B, 0x0A]);
        assert_eq!(&frame[5..7], &[0xEA, 0x05]);
        assert!(frame[7..].iter().all(|byte| *byte == 0));
    }

    #[test]
    fn send_encode_reports_its_own_length_fault() {
        assert_eq!(
            Request::Send {
                token: 1,
                length: 59,
            }
            .encode(),
            Err(NetError::Undersize)
        );
        assert_eq!(
            Request::Send {
                token: 1,
                length: 1515,
            }
            .encode(),
            Err(NetError::Oversize)
        );
    }

    #[test]
    fn send_decode_refuses_out_of_bounds_length() {
        for length in [0u16, 59, 1515, u16::MAX] {
            let mut frame = Request::Send {
                token: 1,
                length: 60,
            }
            .encode()
            .unwrap();
            frame[5..7].copy_from_slice(&length.to_le_bytes());
            assert_eq!(
                Request::decode(&frame),
                Err(NetError::Protocol),
                "length {length} accepted"
            );
        }
    }

    #[test]
    fn request_decode_rejects_unknown_op_bytes() {
        for op in [0u8, 5, 0x0F, 0x10, 0xFF] {
            let mut frame = Request::Poll.encode().unwrap();
            frame[0] = op;
            assert_eq!(Request::decode(&frame), Err(NetError::Protocol));
        }
    }

    #[test]
    fn request_decode_rejects_dirty_tails() {
        for request in [Request::Mac, Request::Poll, Request::Stats] {
            let mut frame = request.encode().unwrap();
            frame[30] = 0x40;
            assert_eq!(Request::decode(&frame), Err(NetError::Protocol));
        }
        let mut frame = Request::Send {
            token: 1,
            length: 60,
        }
        .encode()
        .unwrap();
        frame[7] = 1;
        assert_eq!(Request::decode(&frame), Err(NetError::Protocol));
    }

    #[test]
    fn mac_reply_pins_the_six_address_bytes() {
        let address = [0x52_u8, 0x55, 0x00, 0x12, 0x34, 0x56];
        let frame = Reply::Mac(address).encode().unwrap();
        assert_eq!(frame[0], REPLY_MAC);
        assert_eq!(&frame[1..7], &address);
        assert!(frame[7..].iter().all(|byte| *byte == 0));
        assert_eq!(Reply::decode(&frame), Ok(Reply::Mac(address)));
    }

    #[test]
    fn data_reply_pins_token_and_length_offsets() {
        let frame = Reply::Data {
            token: 0x0102_0304,
            length: 60,
        }
        .encode()
        .unwrap();
        assert_eq!(frame[0], REPLY_DATA);
        assert_eq!(&frame[1..5], &[0x04, 0x03, 0x02, 0x01]);
        assert_eq!(&frame[5..7], &[0x3C, 0x00]);
        assert!(frame[7..].iter().all(|byte| *byte == 0));
        assert_eq!(
            Reply::decode(&frame),
            Ok(Reply::Data {
                token: 0x0102_0304,
                length: 60
            })
        );
    }

    #[test]
    fn stats_reply_pins_the_five_counters() {
        let stats = Stats {
            tx_frames: 10,
            tx_errors: 1,
            rx_frames: 20,
            rx_errors: 2,
            rx_overruns: 3,
        };
        let frame = Reply::Stats(stats).encode().unwrap();
        assert_eq!(frame[0], REPLY_STATS);
        assert_eq!(&frame[1..5], &10u32.to_le_bytes());
        assert_eq!(&frame[5..9], &1u32.to_le_bytes());
        assert_eq!(&frame[9..13], &20u32.to_le_bytes());
        assert_eq!(&frame[13..17], &2u32.to_le_bytes());
        assert_eq!(&frame[17..21], &3u32.to_le_bytes());
        assert!(frame[21..].iter().all(|byte| *byte == 0));
        assert_eq!(Reply::decode(&frame), Ok(Reply::Stats(stats)));
    }

    #[test]
    fn reply_all_variants_roundtrip() {
        let replies = [
            Reply::Ack,
            Reply::Err {
                error: NetError::NoDevice,
            },
            Reply::Err {
                error: NetError::Protocol,
            },
            Reply::Mac([0u8; MAC_BYTES]),
            Reply::Mac([0xFF; MAC_BYTES]),
            Reply::Data {
                token: 0,
                length: 60,
            },
            Reply::Data {
                token: u32::MAX,
                length: 1514,
            },
            Reply::NoFrame,
            Reply::Stats(Stats {
                tx_frames: u32::MAX,
                tx_errors: 0,
                rx_frames: u32::MAX,
                rx_errors: 0,
                rx_overruns: u32::MAX,
            }),
        ];
        for reply in replies {
            assert_eq!(Reply::decode(&reply.encode().unwrap()), Ok(reply));
        }
    }

    #[test]
    fn reply_decode_rejects_unknown_tags_and_dirty_tails() {
        for tag in [0x00u8, 0x0F, 0x16, 0xFF] {
            let mut frame = [0u8; FRAME_BYTES];
            frame[0] = tag;
            assert_eq!(Reply::decode(&frame), Err(NetError::Protocol));
        }
        let cases = [
            Reply::Ack.encode().unwrap(),
            Reply::Err {
                error: NetError::Busy,
            }
            .encode()
            .unwrap(),
            Reply::Mac([1; MAC_BYTES]).encode().unwrap(),
            Reply::Data {
                token: 1,
                length: 60,
            }
            .encode()
            .unwrap(),
            Reply::NoFrame.encode().unwrap(),
            Reply::Stats(Stats {
                tx_frames: 1,
                tx_errors: 0,
                rx_frames: 0,
                rx_errors: 0,
                rx_overruns: 0,
            })
            .encode()
            .unwrap(),
        ];
        for seed in cases {
            let dirty_byte = match seed[0] {
                REPLY_MAC => 7,
                REPLY_DATA => 7,
                REPLY_STATS => 21,
                REPLY_ERROR => 2,
                _ => 1,
            };
            let mut dirty = seed;
            dirty[dirty_byte] = 0x01;
            assert_eq!(Reply::decode(&dirty), Err(NetError::Protocol));
        }
    }

    #[test]
    fn data_reply_decode_refuses_out_of_bounds_length() {
        for length in [0u16, 59, 1515, u16::MAX] {
            let mut frame = Reply::Data {
                token: 1,
                length: 60,
            }
            .encode()
            .unwrap();
            frame[5..7].copy_from_slice(&length.to_le_bytes());
            assert_eq!(
                Reply::decode(&frame),
                Err(NetError::Protocol),
                "length {length} accepted"
            );
        }
    }

    #[test]
    fn wire_error_codes_roundtrip() {
        let codes = [
            NetError::NoDevice,
            NetError::Busy,
            NetError::Oversize,
            NetError::Undersize,
            NetError::Device,
        ];
        for code in codes {
            assert_eq!(NetError::from_byte(code.to_byte()), code);
        }
        assert_eq!(NetError::NoDevice.to_byte(), 1);
        assert_eq!(NetError::Device.to_byte(), 5);
        assert_eq!(NetError::Protocol.to_byte(), 255);
    }

    #[test]
    fn unknown_wire_error_bytes_map_to_protocol() {
        for byte in [0u8, 6, 128, 254] {
            assert_eq!(NetError::from_byte(byte), NetError::Protocol);
        }
    }

    fn seed_frames() -> Vec<[u8; FRAME_BYTES]> {
        let mut frames = Vec::new();
        for request in [
            Request::Mac,
            Request::Send {
                token: 0x00AB_CDEF,
                length: 64,
            },
            Request::Poll,
            Request::Stats,
        ] {
            frames.push(request.encode().expect("seed request encodes"));
        }
        for reply in [
            Reply::Ack,
            Reply::Err {
                error: NetError::Device,
            },
            Reply::Mac([0x52, 0x55, 0x00, 0x12, 0x34, 0x56]),
            Reply::Data {
                token: 9,
                length: 1514,
            },
            Reply::NoFrame,
            Reply::Stats(Stats {
                tx_frames: 3,
                tx_errors: 0,
                rx_frames: 7,
                rx_errors: 1,
                rx_overruns: 0,
            }),
        ] {
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
