use crate::u16_at;

pub const MAC_BYTES: usize = 6;
pub const MIN_FRAME_BYTES: usize = 60;

pub const ETHERTYPE_ARP: u16 = 0x0806;
pub const ETHERTYPE_IPV4: u16 = 0x0800;

pub const BROADCAST: [u8; MAC_BYTES] = [0xff; MAC_BYTES];

pub const HEADER_BYTES: usize = 14;

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum EthernetError {
    TooShort,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct Header<'a> {
    pub destination: [u8; MAC_BYTES],
    pub source: [u8; MAC_BYTES],
    pub ethertype: u16,
    pub payload: &'a [u8],
}

pub fn decode(frame: &[u8]) -> Result<Header<'_>, EthernetError> {
    if frame.len() < HEADER_BYTES {
        return Err(EthernetError::TooShort);
    }
    let mut destination = [0u8; MAC_BYTES];
    destination.copy_from_slice(&frame[0..MAC_BYTES]);
    let mut source = [0u8; MAC_BYTES];
    source.copy_from_slice(&frame[MAC_BYTES..2 * MAC_BYTES]);
    Ok(Header {
        destination,
        source,
        ethertype: u16_at(frame, 12),
        payload: &frame[HEADER_BYTES..],
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arp;

    const MAC: [u8; MAC_BYTES] = [0x02, 0x52, 0x4e, 0x45, 0x32, 0x07];
    const IP: [u8; 4] = [10, 0, 2, 15];
    const PEER_IP: [u8; 4] = [10, 0, 2, 2];

    #[test]
    fn decode_splits_header_and_payload() {
        let frame = arp::request(MAC, IP, PEER_IP);
        let header = decode(&frame).expect("frame decodes");
        assert_eq!(header.destination, BROADCAST);
        assert_eq!(header.source, MAC);
        assert_eq!(header.ethertype, ETHERTYPE_ARP);
        assert_eq!(header.payload.len(), MIN_FRAME_BYTES - 14);
    }

    #[test]
    fn decode_rejects_short_frames() {
        assert_eq!(decode(&[0u8; 13]).unwrap_err(), EthernetError::TooShort);
        let header = decode(&[0u8; 14]).expect("bare header decodes");
        assert!(header.payload.is_empty());
        assert!(decode(&[0u8; 15]).is_ok());
    }

    #[test]
    fn decode_splits_foreign_ethertype_routing() {
        let mut frame = arp::request(MAC, IP, PEER_IP);
        frame[12..14].copy_from_slice(&0x0800u16.to_be_bytes());
        let header = decode(&frame).expect("header decodes");
        assert_eq!(header.ethertype, ETHERTYPE_IPV4);
        assert_ne!(header.ethertype, ETHERTYPE_ARP);
    }

    #[test]
    fn decode_accepts_frame_longer_than_minimum() {
        let mut frame = arp::request(MAC, IP, PEER_IP).to_vec();
        frame.resize(128, 0x5a);
        let header = decode(&frame).expect("frame decodes");
        assert_eq!(header.payload.len(), 128 - HEADER_BYTES);
    }

    #[test]
    fn ethertype_constants_are_pinned() {
        assert_eq!(ETHERTYPE_ARP, 0x0806);
        assert_eq!(ETHERTYPE_IPV4, 0x0800);
        assert_eq!(HEADER_BYTES, 14);
        assert_eq!(MIN_FRAME_BYTES, 60);
        assert_eq!(BROADCAST, [0xff, 0xff, 0xff, 0xff, 0xff, 0xff]);
    }

    #[test]
    fn decode_never_panics() {
        let seeds = [
            arp::request(MAC, IP, PEER_IP).to_vec(),
            arp::reply(MAC, IP, MAC, PEER_IP).to_vec(),
            vec![0u8; 14],
        ];
        for seed in seeds {
            for byte_index in 0..seed.len().min(16) {
                for bit in 0..8 {
                    let mut mutated = seed.clone();
                    mutated[byte_index] ^= 1 << bit;
                    let _ = decode(&mutated);
                }
            }
        }
    }
}
