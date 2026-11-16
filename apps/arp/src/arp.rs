pub const MAC_BYTES: usize = 6;
pub const IPV4_BYTES: usize = 4;
pub const MIN_FRAME_BYTES: usize = 60;

pub const ETHERTYPE_ARP: u16 = 0x0806;
pub const ETHERTYPE_IPV4: u16 = 0x0800;

pub const BROADCAST: [u8; MAC_BYTES] = [0xff; MAC_BYTES];

const ETHER_HEADER_BYTES: usize = 14;
const ARP_BYTES: usize = 28;

const ARP_HRD_ETHERNET: u16 = 0x0001;

const ARP_HRD_AT: usize = 0;
const ARP_PRO_AT: usize = 2;
const ARP_HLN_AT: usize = 4;
const ARP_PLN_AT: usize = 5;
const ARP_OPER_AT: usize = 6;
const ARP_SHA_AT: usize = 8;
const ARP_SPA_AT: usize = 14;
const ARP_THA_AT: usize = 18;
const ARP_TPA_AT: usize = 24;

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum ArpError {
    TooShort,
    HardwareFormat,
    ProtocolFormat,
    AddressSize,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum Operation {
    Request,
    Reply,
    Other(u16),
}

impl Operation {
    pub fn from_code(code: u16) -> Operation {
        match code {
            1 => Operation::Request,
            2 => Operation::Reply,
            other => Operation::Other(other),
        }
    }

    pub fn code(self) -> u16 {
        match self {
            Operation::Request => 1,
            Operation::Reply => 2,
            Operation::Other(other) => other,
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct EthernetHeader<'a> {
    pub destination: [u8; MAC_BYTES],
    pub source: [u8; MAC_BYTES],
    pub ethertype: u16,
    pub payload: &'a [u8],
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct ArpPacket {
    pub operation: Operation,
    pub sender_mac: [u8; MAC_BYTES],
    pub sender_ip: [u8; IPV4_BYTES],
    pub target_mac: [u8; MAC_BYTES],
    pub target_ip: [u8; IPV4_BYTES],
}

fn u16_at(bytes: &[u8], at: usize) -> u16 {
    u16::from_be_bytes([bytes[at], bytes[at + 1]])
}

pub fn decode_ethernet(frame: &[u8]) -> Result<EthernetHeader<'_>, ArpError> {
    if frame.len() < ETHER_HEADER_BYTES {
        return Err(ArpError::TooShort);
    }
    let mut destination = [0u8; MAC_BYTES];
    destination.copy_from_slice(&frame[0..MAC_BYTES]);
    let mut source = [0u8; MAC_BYTES];
    source.copy_from_slice(&frame[MAC_BYTES..2 * MAC_BYTES]);
    Ok(EthernetHeader {
        destination,
        source,
        ethertype: u16_at(frame, 12),
        payload: &frame[ETHER_HEADER_BYTES..],
    })
}

pub fn decode_arp(payload: &[u8]) -> Result<ArpPacket, ArpError> {
    if payload.len() < ARP_BYTES {
        return Err(ArpError::TooShort);
    }
    if u16_at(payload, ARP_HRD_AT) != ARP_HRD_ETHERNET {
        return Err(ArpError::HardwareFormat);
    }
    if u16_at(payload, ARP_PRO_AT) != ETHERTYPE_IPV4 {
        return Err(ArpError::ProtocolFormat);
    }
    if payload[ARP_HLN_AT] != MAC_BYTES as u8 || payload[ARP_PLN_AT] != IPV4_BYTES as u8 {
        return Err(ArpError::AddressSize);
    }
    let mut sender_mac = [0u8; MAC_BYTES];
    sender_mac.copy_from_slice(&payload[ARP_SHA_AT..ARP_SHA_AT + MAC_BYTES]);
    let mut sender_ip = [0u8; IPV4_BYTES];
    sender_ip.copy_from_slice(&payload[ARP_SPA_AT..ARP_SPA_AT + IPV4_BYTES]);
    let mut target_mac = [0u8; MAC_BYTES];
    target_mac.copy_from_slice(&payload[ARP_THA_AT..ARP_THA_AT + MAC_BYTES]);
    let mut target_ip = [0u8; IPV4_BYTES];
    target_ip.copy_from_slice(&payload[ARP_TPA_AT..ARP_TPA_AT + IPV4_BYTES]);
    Ok(ArpPacket {
        operation: Operation::from_code(u16_at(payload, ARP_OPER_AT)),
        sender_mac,
        sender_ip,
        target_mac,
        target_ip,
    })
}

fn encode_into_frame(destination: [u8; MAC_BYTES], packet: &ArpPacket) -> [u8; MIN_FRAME_BYTES] {
    let mut frame = [0u8; MIN_FRAME_BYTES];
    frame[0..MAC_BYTES].copy_from_slice(&destination);
    frame[MAC_BYTES..2 * MAC_BYTES].copy_from_slice(&packet.sender_mac);
    frame[12..14].copy_from_slice(&ETHERTYPE_ARP.to_be_bytes());
    let arp = &mut frame[ETHER_HEADER_BYTES..ETHER_HEADER_BYTES + ARP_BYTES];
    arp[ARP_HRD_AT..ARP_HRD_AT + 2].copy_from_slice(&ARP_HRD_ETHERNET.to_be_bytes());
    arp[ARP_PRO_AT..ARP_PRO_AT + 2].copy_from_slice(&ETHERTYPE_IPV4.to_be_bytes());
    arp[ARP_HLN_AT] = MAC_BYTES as u8;
    arp[ARP_PLN_AT] = IPV4_BYTES as u8;
    arp[ARP_OPER_AT..ARP_OPER_AT + 2].copy_from_slice(&packet.operation.code().to_be_bytes());
    arp[ARP_SHA_AT..ARP_SHA_AT + MAC_BYTES].copy_from_slice(&packet.sender_mac);
    arp[ARP_SPA_AT..ARP_SPA_AT + IPV4_BYTES].copy_from_slice(&packet.sender_ip);
    arp[ARP_THA_AT..ARP_THA_AT + MAC_BYTES].copy_from_slice(&packet.target_mac);
    arp[ARP_TPA_AT..ARP_TPA_AT + IPV4_BYTES].copy_from_slice(&packet.target_ip);
    frame
}

pub fn request(sender_mac: [u8; MAC_BYTES], sender_ip: [u8; IPV4_BYTES], target_ip: [u8; IPV4_BYTES]) -> [u8; MIN_FRAME_BYTES] {
    let packet = ArpPacket {
        operation: Operation::Request,
        sender_mac,
        sender_ip,
        target_mac: [0u8; MAC_BYTES],
        target_ip,
    };
    encode_into_frame(BROADCAST, &packet)
}

pub fn reply(
    sender_mac: [u8; MAC_BYTES],
    sender_ip: [u8; IPV4_BYTES],
    target_mac: [u8; MAC_BYTES],
    target_ip: [u8; IPV4_BYTES],
) -> [u8; MIN_FRAME_BYTES] {
    let packet = ArpPacket {
        operation: Operation::Reply,
        sender_mac,
        sender_ip,
        target_mac,
        target_ip,
    };
    encode_into_frame(target_mac, &packet)
}

pub fn gratuitous_reply(sender_mac: [u8; MAC_BYTES], sender_ip: [u8; IPV4_BYTES]) -> [u8; MIN_FRAME_BYTES] {
    let packet = ArpPacket {
        operation: Operation::Reply,
        sender_mac,
        sender_ip,
        target_mac: sender_mac,
        target_ip: sender_ip,
    };
    encode_into_frame(BROADCAST, &packet)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAC: [u8; MAC_BYTES] = [0x02, 0x52, 0x4e, 0x45, 0x32, 0x07];
    const PEER_MAC: [u8; MAC_BYTES] = [0x52, 0x55, 0x0a, 0x00, 0x02, 0x00];
    const IP: [u8; IPV4_BYTES] = [10, 0, 2, 15];
    const PEER_IP: [u8; IPV4_BYTES] = [10, 0, 2, 2];

    fn arp_payload(packet_bytes: &[u8]) -> Vec<u8> {
        let mut payload = vec![0x00, 0x01, 0x08, 0x00, 0x06, 0x04];
        payload.extend_from_slice(packet_bytes);
        payload
    }

    #[test]
    fn request_wire_image_is_classic_who_has() {
        let frame = request(MAC, IP, PEER_IP);
        let expected: [u8; MIN_FRAME_BYTES] = [
            0xff, 0xff, 0xff, 0xff, 0xff, 0xff, //
            0x02, 0x52, 0x4e, 0x45, 0x32, 0x07, //
            0x08, 0x06, //
            0x00, 0x01, 0x08, 0x00, 0x06, 0x04, 0x00, 0x01, //
            0x02, 0x52, 0x4e, 0x45, 0x32, 0x07, //
            10, 0, 2, 15, //
            0, 0, 0, 0, 0, 0, //
            10, 0, 2, 2, //
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        ];
        assert_eq!(frame, expected);
    }

    #[test]
    fn reply_wire_image_is_unicast_is_at() {
        let frame = reply(PEER_MAC, PEER_IP, MAC, IP);
        assert_eq!(&frame[0..6], &MAC);
        assert_eq!(&frame[6..12], &PEER_MAC);
        assert_eq!(&frame[12..14], &[0x08, 0x06]);
        assert_eq!(&frame[20..22], &[0x00, 0x02]);
    }

    #[test]
    fn gratuitous_reply_announces_sender_to_broadcast() {
        let frame = gratuitous_reply(MAC, IP);
        assert_eq!(&frame[0..6], &BROADCAST);
        assert_eq!(&frame[6..12], &MAC);
        assert_eq!(&frame[20..22], &[0x00, 0x02]);
        assert_eq!(&frame[32..38], &MAC);
        assert_eq!(&frame[38..42], &IP);
    }

    #[test]
    fn decode_round_trips_request() {
        let frame = request(MAC, IP, PEER_IP);
        let header = decode_ethernet(&frame).expect("frame decodes");
        assert_eq!(header.ethertype, ETHERTYPE_ARP);
        let packet = decode_arp(header.payload).expect("arp decodes");
        assert_eq!(packet.operation, Operation::Request);
        assert_eq!(packet.sender_mac, MAC);
        assert_eq!(packet.sender_ip, IP);
        assert_eq!(packet.target_mac, [0u8; MAC_BYTES]);
        assert_eq!(packet.target_ip, PEER_IP);
    }

    #[test]
    fn decode_round_trips_reply() {
        let frame = reply(PEER_MAC, PEER_IP, MAC, IP);
        let header = decode_ethernet(&frame).expect("frame decodes");
        assert_eq!(header.ethertype, ETHERTYPE_ARP);
        let packet = decode_arp(header.payload).expect("arp decodes");
        assert_eq!(packet.operation, Operation::Reply);
        assert_eq!(packet.sender_mac, PEER_MAC);
        assert_eq!(packet.sender_ip, PEER_IP);
        assert_eq!(packet.target_mac, MAC);
        assert_eq!(packet.target_ip, IP);
    }

    #[test]
    fn decode_ethernet_splits_header_and_payload() {
        let frame = request(MAC, IP, PEER_IP);
        let header = decode_ethernet(&frame).expect("frame decodes");
        assert_eq!(header.destination, BROADCAST);
        assert_eq!(header.source, MAC);
        assert_eq!(header.ethertype, ETHERTYPE_ARP);
        assert_eq!(header.payload.len(), MIN_FRAME_BYTES - 14);
    }

    #[test]
    fn decode_rejects_short_frames() {
        assert_eq!(decode_ethernet(&[0u8; 13]).unwrap_err(), ArpError::TooShort);
        assert_eq!(decode_arp(&arp_payload(&[0u8; 20])).unwrap_err(), ArpError::TooShort);
    }

    #[test]
    fn decode_splits_foreign_ethertype_routing() {
        let mut frame = request(MAC, IP, PEER_IP);
        frame[12..14].copy_from_slice(&0x0800u16.to_be_bytes());
        let header = decode_ethernet(&frame).expect("header decodes");
        assert_ne!(header.ethertype, ETHERTYPE_ARP);
    }

    #[test]
    fn decode_rejects_foreign_hardware_format() {
        let mut frame = request(MAC, IP, PEER_IP);
        frame[14..16].copy_from_slice(&[0x00, 0x0f]);
        let header = decode_ethernet(&frame).expect("header decodes");
        assert_eq!(decode_arp(header.payload).unwrap_err(), ArpError::HardwareFormat);
    }

    #[test]
    fn decode_rejects_non_ipv4_protocol() {
        let mut frame = request(MAC, IP, PEER_IP);
        frame[16..18].copy_from_slice(&[0x86, 0xdd]);
        let header = decode_ethernet(&frame).expect("header decodes");
        assert_eq!(decode_arp(header.payload).unwrap_err(), ArpError::ProtocolFormat);
    }

    #[test]
    fn decode_rejects_bad_address_sizes() {
        let mut frame = request(MAC, IP, PEER_IP);
        frame[18] = 8;
        let header = decode_ethernet(&frame).expect("header decodes");
        assert_eq!(decode_arp(header.payload).unwrap_err(), ArpError::AddressSize);
        let mut frame = request(MAC, IP, PEER_IP);
        frame[19] = 6;
        let header = decode_ethernet(&frame).expect("header decodes");
        assert_eq!(decode_arp(header.payload).unwrap_err(), ArpError::AddressSize);
    }

    #[test]
    fn decode_passes_unknown_operations_through() {
        let mut frame = request(MAC, IP, PEER_IP);
        frame[20..22].copy_from_slice(&[0x00, 0x09]);
        let header = decode_ethernet(&frame).expect("header decodes");
        let packet = decode_arp(header.payload).expect("unknown operation still decodes");
        assert_eq!(packet.operation, Operation::Other(9));
    }

    #[test]
    fn decode_accepts_frame_longer_than_minimum() {
        let mut frame = request(MAC, IP, PEER_IP).to_vec();
        frame.resize(128, 0x5a);
        let header = decode_ethernet(&frame).expect("frame decodes");
        let packet = decode_arp(header.payload).expect("padding after arp is ignored");
        assert_eq!(packet.sender_ip, IP);
    }

    #[test]
    fn operation_code_mapping_is_total() {
        assert_eq!(Operation::from_code(1), Operation::Request);
        assert_eq!(Operation::from_code(2), Operation::Reply);
        assert_eq!(Operation::from_code(0), Operation::Other(0));
        assert_eq!(Operation::from_code(u16::MAX), Operation::Other(u16::MAX));
        assert_eq!(Operation::Request.code(), 1);
        assert_eq!(Operation::Reply.code(), 2);
        assert_eq!(Operation::Other(7).code(), 7);
    }
}
