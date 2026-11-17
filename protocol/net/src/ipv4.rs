use crate::u16_at;
use alloc::vec::Vec;

pub const IPV4_BYTES: usize = 4;
pub const HEADER_BYTES: usize = 20;
pub const MAX_DATAGRAM_BYTES: usize = u16::MAX as usize;
pub const DEFAULT_TTL: u8 = 64;

pub const PROTO_ICMP: u8 = 1;

const VERSION: u8 = 4;
const FLAGS_MORE_FRAGMENTS: u16 = 0x2000;
const FRAGMENT_OFFSET_MASK: u16 = 0x1FFF;

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum IpError {
    TooShort,
    BadVersion,
    BadHeaderLength,
    Truncated,
    BadChecksum,
    Oversize,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct Packet<'a> {
    pub source: [u8; IPV4_BYTES],
    pub destination: [u8; IPV4_BYTES],
    pub protocol: u8,
    pub more_fragments: bool,
    pub fragment_offset: u16,
    pub payload: &'a [u8],
}

pub fn checksum(block: &[u8]) -> u16 {
    let mut sum: u32 = 0;
    let mut words = block.chunks_exact(2);
    for word in words.by_ref() {
        sum += u16::from_be_bytes([word[0], word[1]]) as u32;
    }
    let odd = words.remainder();
    if !odd.is_empty() {
        sum += (odd[0] as u32) << 8;
    }
    while sum >> 16 != 0 {
        sum = (sum & 0xFFFF) + (sum >> 16);
    }
    !(sum as u16)
}

pub fn decode(datagram: &[u8]) -> Result<Packet<'_>, IpError> {
    if datagram.len() < HEADER_BYTES {
        return Err(IpError::TooShort);
    }
    if datagram[0] >> 4 != VERSION {
        return Err(IpError::BadVersion);
    }
    let header_len = (datagram[0] & 0x0F) as usize * 4;
    if header_len < HEADER_BYTES || header_len > datagram.len() {
        return Err(IpError::BadHeaderLength);
    }
    let total_len = u16_at(datagram, 2) as usize;
    if total_len < header_len || total_len > datagram.len() {
        return Err(IpError::Truncated);
    }
    if checksum(&datagram[..header_len]) != 0 {
        return Err(IpError::BadChecksum);
    }
    let flags_and_offset = u16_at(datagram, 6);
    let mut source = [0u8; IPV4_BYTES];
    source.copy_from_slice(&datagram[12..16]);
    let mut destination = [0u8; IPV4_BYTES];
    destination.copy_from_slice(&datagram[16..20]);
    Ok(Packet {
        source,
        destination,
        protocol: datagram[9],
        more_fragments: flags_and_offset & FLAGS_MORE_FRAGMENTS != 0,
        fragment_offset: flags_and_offset & FRAGMENT_OFFSET_MASK,
        payload: &datagram[header_len..total_len],
    })
}

pub fn encode(
    source: [u8; IPV4_BYTES],
    destination: [u8; IPV4_BYTES],
    protocol: u8,
    payload: &[u8],
    out: &mut Vec<u8>,
) -> Result<(), IpError> {
    let total_len = HEADER_BYTES + payload.len();
    if total_len > MAX_DATAGRAM_BYTES {
        return Err(IpError::Oversize);
    }
    let start = out.len();
    out.resize(start + HEADER_BYTES, 0);
    out[start] = (VERSION << 4) | (HEADER_BYTES / 4) as u8;
    out[start + 2] = (total_len >> 8) as u8;
    out[start + 3] = total_len as u8;
    out[start + 8] = DEFAULT_TTL;
    out[start + 9] = protocol;
    out[start + 12..start + 16].copy_from_slice(&source);
    out[start + 16..start + 20].copy_from_slice(&destination);
    let header_sum = checksum(&out[start..start + HEADER_BYTES]);
    out[start + 10..start + 12].copy_from_slice(&header_sum.to_be_bytes());
    out.extend_from_slice(payload);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SRC: [u8; IPV4_BYTES] = [10, 0, 2, 15];
    const DST: [u8; IPV4_BYTES] = [10, 0, 2, 2];

    fn encoded_sample() -> Vec<u8> {
        let mut datagram = Vec::new();
        encode(SRC, DST, PROTO_ICMP, &[0xaa], &mut datagram).expect("sample encodes");
        datagram
    }

    fn reseal(datagram: &mut [u8], header_len: usize) {
        datagram[10..12].copy_from_slice(&[0, 0]);
        let sum = checksum(&datagram[..header_len]);
        datagram[10..12].copy_from_slice(&sum.to_be_bytes());
    }

    #[test]
    fn encode_wire_image_is_pinned() {
        let expected: [u8; 21] = [
            0x45, 0x00, 0x00, 0x15, 0x00, 0x00, 0x00, 0x00, 0x40, 0x01, 0x62, 0xd8, //
            10, 0, 2, 15, 10, 0, 2, 2, 0xaa,
        ];
        assert_eq!(encoded_sample().as_slice(), &expected);
    }

    #[test]
    fn checksum_of_valid_header_is_zero() {
        assert_eq!(checksum(&encoded_sample()[..HEADER_BYTES]), 0);
    }

    #[test]
    fn checksum_covers_odd_tail() {
        assert_eq!(checksum(&[0x00, 0x00, 0xff]), !(0x0000u16 + 0xff00u16));
        assert_eq!(checksum(&[]), 0xffff);
    }

    #[test]
    fn decode_round_trips_encoder_output() {
        let datagram = encoded_sample();
        let packet = decode(&datagram).expect("datagram decodes");
        assert_eq!(packet.source, SRC);
        assert_eq!(packet.destination, DST);
        assert_eq!(packet.protocol, PROTO_ICMP);
        assert_eq!(packet.payload, &[0xaa]);
        assert!(!packet.more_fragments);
        assert_eq!(packet.fragment_offset, 0);
    }

    #[test]
    fn decode_trims_padding_beyond_total_length() {
        let mut datagram = encoded_sample();
        datagram.extend_from_slice(&[0x5a; 10]);
        let packet = decode(&datagram).expect("padded datagram decodes");
        assert_eq!(packet.payload, &[0xaa]);
    }

    #[test]
    fn decode_rejects_short_datagrams() {
        assert_eq!(decode(&[0u8; 19]).unwrap_err(), IpError::TooShort);
        let mut datagram = encoded_sample();
        datagram.truncate(19);
        assert_eq!(decode(&datagram).unwrap_err(), IpError::TooShort);
    }

    #[test]
    fn decode_rejects_foreign_versions() {
        for version in [4u8, 5, 6, 15] {
            let mut datagram = encoded_sample();
            datagram[0] = (version << 4) | 5;
            reseal(&mut datagram, HEADER_BYTES);
            if version == VERSION {
                assert!(decode(&datagram).is_ok());
            } else {
                assert_eq!(decode(&datagram).unwrap_err(), IpError::BadVersion);
            }
        }
    }

    #[test]
    fn decode_rejects_bad_header_lengths() {
        for ihl in [0u8, 1, 4, 15] {
            let mut datagram = encoded_sample();
            datagram[0] = (VERSION << 4) | ihl;
            reseal(&mut datagram, HEADER_BYTES);
            assert_eq!(
                decode(&datagram).unwrap_err(),
                IpError::BadHeaderLength,
                "ihl {ihl} accepted"
            );
        }
    }

    #[test]
    fn decode_skips_header_options() {
        let mut datagram = encoded_sample();
        let options = [0x01, 0x01, 0x00, 0x00];
        datagram.splice(HEADER_BYTES..HEADER_BYTES, options);
        datagram[0] = (VERSION << 4) | 6;
        let total = HEADER_BYTES + 4 + 1;
        datagram[2..4].copy_from_slice(&(total as u16).to_be_bytes());
        reseal(&mut datagram, HEADER_BYTES + 4);
        let packet = decode(&datagram).expect("options skipped");
        assert_eq!(packet.payload, &[0xaa]);
    }

    #[test]
    fn decode_rejects_inconsistent_total_lengths() {
        let mut datagram = encoded_sample();
        datagram[2..4].copy_from_slice(&(19u16).to_be_bytes());
        reseal(&mut datagram, HEADER_BYTES);
        assert_eq!(decode(&datagram).unwrap_err(), IpError::Truncated);

        let mut datagram = encoded_sample();
        datagram[2..4].copy_from_slice(&(30u16).to_be_bytes());
        reseal(&mut datagram, HEADER_BYTES);
        assert_eq!(decode(&datagram).unwrap_err(), IpError::Truncated);
    }

    #[test]
    fn decode_rejects_corrupted_checksums() {
        let neutral_bytes = [
            1usize, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19,
        ];
        for byte in neutral_bytes {
            let mut datagram = encoded_sample();
            datagram[byte] ^= 0x01;
            assert_eq!(
                decode(&datagram).unwrap_err(),
                IpError::BadChecksum,
                "byte {byte} corruption survived"
            );
        }
    }

    #[test]
    fn decode_exposes_fragment_fields() {
        let mut datagram = encoded_sample();
        datagram[6..8].copy_from_slice(&[0x20, 0x03]);
        reseal(&mut datagram, HEADER_BYTES);
        let packet = decode(&datagram).expect("fragments decode");
        assert!(packet.more_fragments);
        assert_eq!(packet.fragment_offset, 3);
    }

    #[test]
    fn encode_refuses_datagrams_beyond_u16_total_length() {
        let oversized = vec![0u8; MAX_DATAGRAM_BYTES - HEADER_BYTES + 1];
        let mut out = Vec::new();
        assert_eq!(
            encode(SRC, DST, PROTO_ICMP, &oversized, &mut out),
            Err(IpError::Oversize)
        );
        assert!(out.is_empty());
        let mut fits = vec![0u8; MAX_DATAGRAM_BYTES - HEADER_BYTES];
        fits[0] = 0x42;
        assert!(encode(SRC, DST, PROTO_ICMP, &fits, &mut out).is_ok());
    }

    #[test]
    fn encode_appends_to_existing_bytes() {
        let mut out = vec![0xde, 0xad];
        encode(SRC, DST, PROTO_ICMP, &[1, 2], &mut out).expect("appends");
        assert_eq!(&out[..2], &[0xde, 0xad]);
        let packet = decode(&out[2..]).expect("appended datagram decodes");
        assert_eq!(packet.payload, &[1, 2]);
    }

    #[test]
    fn decode_never_panics() {
        let mut seeds = vec![
            encoded_sample(),
            vec![0x45, 0, 0, 20, 0, 0, 0, 0, 64, 1, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2],
            vec![0u8; 20],
        ];
        for seed in seeds.iter_mut() {
            let sum = checksum(&seed[..HEADER_BYTES.min(seed.len())]);
            if seed.len() >= HEADER_BYTES {
                seed[10..12].copy_from_slice(&sum.to_be_bytes());
            }
        }
        for seed in seeds {
            for byte in 0..seed.len().min(HEADER_BYTES + 2) {
                for bit in 0..8 {
                    let mut mutated = seed.clone();
                    mutated[byte] ^= 1 << bit;
                    let _ = decode(&mutated);
                }
            }
        }
    }
}
