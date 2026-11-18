use crate::ipv4::checksum;
use alloc::vec;
use alloc::vec::Vec;

pub const ECHO_REPLY: u8 = 0;
pub const ECHO_REQUEST: u8 = 8;

const HEADER_BYTES: usize = 4;
const ECHO_BYTES: usize = 8;

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum IcmpError {
    TooShort,
    BadChecksum,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum Icmp<'a> {
    Echo {
        reply: bool,
        id: u16,
        sequence: u16,
        data: &'a [u8],
    },
    Other {
        kind: u8,
        code: u8,
    },
}

pub fn echo(kind: u8, id: u16, sequence: u16, payload: &[u8]) -> Vec<u8> {
    let mut message = vec![kind, 0, 0, 0];
    message.extend_from_slice(&id.to_be_bytes());
    message.extend_from_slice(&sequence.to_be_bytes());
    message.extend_from_slice(payload);
    let sum = checksum(&message);
    message[2..4].copy_from_slice(&sum.to_be_bytes());
    message
}

pub fn decode(message: &[u8]) -> Result<Icmp<'_>, IcmpError> {
    if message.len() < HEADER_BYTES {
        return Err(IcmpError::TooShort);
    }
    if checksum(message) != 0 {
        return Err(IcmpError::BadChecksum);
    }
    let kind = message[0];
    let code = message[1];
    if (kind == ECHO_REQUEST || kind == ECHO_REPLY) && message.len() >= ECHO_BYTES {
        return Ok(Icmp::Echo {
            reply: kind == ECHO_REPLY,
            id: u16::from_be_bytes([message[4], message[5]]),
            sequence: u16::from_be_bytes([message[6], message[7]]),
            data: &message[ECHO_BYTES..],
        });
    }
    Ok(Icmp::Other { kind, code })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn echo_request_wire_image_is_pinned() {
        let expected: [u8; 9] = [0x08, 0x00, 0xa3, 0xca, 0x12, 0x34, 0x00, 0x01, 0x42];
        assert_eq!(echo(ECHO_REQUEST, 0x1234, 1, &[0x42]).as_slice(), &expected);
    }

    #[test]
    fn checksum_of_valid_message_is_zero() {
        assert_eq!(checksum(&echo(ECHO_REQUEST, 7, 8, b"rosx")), 0);
        assert_eq!(checksum(&echo(ECHO_REPLY, 7, 8, &[])), 0);
    }

    #[test]
    fn decode_round_trips_echo_request_and_reply() {
        let request = echo(ECHO_REQUEST, 0xabcd, 3, b"ping");
        match decode(&request).expect("request decodes") {
            Icmp::Echo {
                reply,
                id,
                sequence,
                data,
            } => {
                assert!(!reply);
                assert_eq!(id, 0xabcd);
                assert_eq!(sequence, 3);
                assert_eq!(data, b"ping");
            }
            Icmp::Other { kind, .. } => panic!("request decoded as kind {kind}"),
        }
        let reply_message = echo(ECHO_REPLY, 0xabcd, 3, b"pong");
        match decode(&reply_message).expect("reply decodes") {
            Icmp::Echo {
                reply,
                id,
                sequence,
                data,
            } => {
                assert!(reply);
                assert_eq!(id, 0xabcd);
                assert_eq!(sequence, 3);
                assert_eq!(data, b"pong");
            }
            Icmp::Other { kind, .. } => panic!("reply decoded as kind {kind}"),
        }
    }

    #[test]
    fn decode_rejects_short_messages() {
        assert_eq!(decode(&[0x08, 0x00]).unwrap_err(), IcmpError::TooShort);
        assert_eq!(decode(&[]).unwrap_err(), IcmpError::TooShort);
    }

    #[test]
    fn decode_rejects_bad_checksums() {
        let mut message = echo(ECHO_REQUEST, 1, 1, b"x");
        message[8] ^= 0x01;
        assert_eq!(decode(&message).unwrap_err(), IcmpError::BadChecksum);
        for byte in 0..4 {
            let mut message = echo(ECHO_REQUEST, 1, 1, b"x");
            message[byte] ^= 0x01;
            assert_eq!(decode(&message).unwrap_err(), IcmpError::BadChecksum);
        }
    }

    #[test]
    fn decode_passes_unknown_kinds_through() {
        let mut message = vec![3, 4, 0, 0];
        let sum = checksum(&message);
        message[2..4].copy_from_slice(&sum.to_be_bytes());
        assert_eq!(decode(&message), Ok(Icmp::Other { kind: 3, code: 4 }));
    }

    #[test]
    fn decode_refuses_truncated_echo_bodies_as_other() {
        let mut message = vec![ECHO_REQUEST, 0, 0, 0, 0x12, 0x34];
        let sum = checksum(&message);
        message[2..4].copy_from_slice(&sum.to_be_bytes());
        assert_eq!(
            decode(&message),
            Ok(Icmp::Other {
                kind: ECHO_REQUEST,
                code: 0
            })
        );
    }

    #[test]
    fn echo_carries_empty_payload() {
        let message = echo(ECHO_REQUEST, 9, 9, &[]);
        assert_eq!(message.len(), ECHO_BYTES);
        match decode(&message).expect("bare echo decodes") {
            Icmp::Echo { data, .. } => assert!(data.is_empty()),
            Icmp::Other { .. } => panic!("bare echo decoded as Other"),
        }
    }

    #[test]
    fn decode_never_panics() {
        let seeds = [
            echo(ECHO_REQUEST, 1, 2, b"abc"),
            echo(ECHO_REPLY, 1, 2, &[]),
            vec![0u8; 4],
        ];
        for seed in seeds {
            for byte in 0..seed.len() {
                for bit in 0..8 {
                    let mut mutated = seed.clone();
                    mutated[byte] ^= 1 << bit;
                    let _ = decode(&mutated);
                }
            }
        }
    }
}
