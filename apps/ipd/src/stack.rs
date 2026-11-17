use alloc::vec::Vec;
use net_protocol::arp::{self, Operation};
use net_protocol::ethernet::{self, MAC_BYTES};
use net_protocol::icmp::{self, Icmp};
use net_protocol::ipv4::{self, IPV4_BYTES};

use crate::arp_cache::{ArpCache, Want};
use crate::device::FrameDevice;

pub const MAX_IP_DATAGRAM_BYTES: usize = 1500;
pub const MAX_WAITING_FRAMES: usize = 8;

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct Config {
    pub address: [u8; IPV4_BYTES],
    pub prefix_len: u8,
    pub gateway: Option<[u8; IPV4_BYTES]>,
}

#[derive(Debug, Copy, Clone, Default, PartialEq, Eq)]
pub struct Counters {
    pub rx_frames: u32,
    pub tx_frames: u32,
    pub rx_dropped: u32,
    pub tx_dropped: u32,
    pub arp_answers: u32,
    pub echo_answers: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    Datagram {
        source: [u8; IPV4_BYTES],
        protocol: u8,
        payload: Vec<u8>,
    },
    Unreachable {
        destination: [u8; IPV4_BYTES],
    },
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum SendError {
    NotConfigured,
    NoRoute,
    Oversize,
    Busy,
}

struct Waiting {
    next_hop: [u8; IPV4_BYTES],
    destination: [u8; IPV4_BYTES],
    datagram: Vec<u8>,
}

pub struct Stack<D: FrameDevice> {
    device: D,
    config: Option<Config>,
    arp: ArpCache,
    waiting: Vec<Waiting>,
    counters: Counters,
}

impl<D: FrameDevice> Stack<D> {
    pub fn new(device: D) -> Stack<D> {
        Stack {
            device,
            config: None,
            arp: ArpCache::new(),
            waiting: Vec::new(),
            counters: Counters::default(),
        }
    }

    pub fn device(&self) -> &D {
        &self.device
    }

    pub fn mac(&self) -> [u8; MAC_BYTES] {
        self.device.mac()
    }

    pub fn config(&self) -> Option<Config> {
        self.config
    }

    pub fn counters(&self) -> Counters {
        self.counters
    }

    pub fn configure(&mut self, config: Config) {
        self.config = Some(config);
    }

    pub fn on_frame(&mut self, frame: &[u8], now_ms: u64) -> Vec<Event> {
        self.counters.rx_frames += 1;
        let Ok(header) = ethernet::decode(frame) else {
            self.counters.rx_dropped += 1;
            return Vec::new();
        };
        let own_mac = self.device.mac();
        if header.destination != own_mac && header.destination != ethernet::BROADCAST {
            self.counters.rx_dropped += 1;
            return Vec::new();
        }
        match header.ethertype {
            ethernet::ETHERTYPE_ARP => {
                self.on_arp(header.payload, now_ms);
                Vec::new()
            }
            ethernet::ETHERTYPE_IPV4 => self.on_ipv4(&header),
            _ => {
                self.counters.rx_dropped += 1;
                Vec::new()
            }
        }
    }

    fn on_arp(&mut self, payload: &[u8], now_ms: u64) {
        let Ok(packet) = arp::decode(payload) else {
            self.counters.rx_dropped += 1;
            return;
        };
        if packet.operation != Operation::Request && packet.operation != Operation::Reply {
            self.counters.rx_dropped += 1;
            return;
        }
        self.arp.learn(packet.sender_ip, packet.sender_mac, now_ms);
        if packet.operation == Operation::Request && self.owns(packet.target_ip) {
            let reply = arp::reply(self.device.mac(), packet.target_ip, packet.sender_mac, packet.sender_ip);
            self.counters.arp_answers += 1;
            self.counters.tx_frames += 1;
            self.device.transmit(&reply);
        }
        self.flush_next_hop(packet.sender_ip, now_ms);
    }

    fn on_ipv4(&mut self, header: &ethernet::Header) -> Vec<Event> {
        let Ok(packet) = ipv4::decode(header.payload) else {
            self.counters.rx_dropped += 1;
            return Vec::new();
        };
        if packet.more_fragments || packet.fragment_offset != 0 {
            self.counters.rx_dropped += 1;
            return Vec::new();
        }
        let Some(config) = self.config else {
            self.counters.rx_dropped += 1;
            return Vec::new();
        };
        if !owns_address(&config, packet.destination) {
            self.counters.rx_dropped += 1;
            return Vec::new();
        }
        if packet.protocol == ipv4::PROTO_ICMP && packet.destination == config.address {
            if let Ok(Icmp::Echo {
                reply: false,
                id,
                sequence,
                data,
            }) = icmp::decode(packet.payload)
            {
                self.echo_answer(header.source, packet.source, config.address, id, sequence, data);
            }
        }
        alloc::vec![Event::Datagram {
            source: packet.source,
            protocol: packet.protocol,
            payload: packet.payload.to_vec(),
        }]
    }

    fn echo_answer(
        &mut self,
        peer_mac: [u8; MAC_BYTES],
        peer_ip: [u8; IPV4_BYTES],
        own_ip: [u8; IPV4_BYTES],
        id: u16,
        sequence: u16,
        data: &[u8],
    ) {
        let message = icmp::echo(icmp::ECHO_REPLY, id, sequence, data);
        let mut datagram = Vec::new();
        if ipv4::encode(own_ip, peer_ip, ipv4::PROTO_ICMP, &message, &mut datagram).is_err() {
            self.counters.tx_dropped += 1;
            return;
        }
        self.transmit_to(peer_mac, &datagram);
        self.counters.echo_answers += 1;
    }

    pub fn tick(&mut self, now_ms: u64) -> Vec<Event> {
        let deadlines = self.arp.deadlines(now_ms);
        if let Some(config) = self.config {
            for target in deadlines.retry {
                let request = arp::request(self.device.mac(), config.address, target);
                self.counters.tx_frames += 1;
                self.device.transmit(&request);
            }
        }
        let mut events = Vec::new();
        for target in deadlines.exhausted {
            let mut parked = Vec::new();
            core::mem::swap(&mut parked, &mut self.waiting);
            for entry in parked {
                if entry.next_hop == target {
                    self.counters.tx_dropped += 1;
                    events.push(Event::Unreachable {
                        destination: entry.destination,
                    });
                } else {
                    self.waiting.push(entry);
                }
            }
        }
        events
    }

    pub fn send_ip(
        &mut self,
        destination: [u8; IPV4_BYTES],
        protocol: u8,
        payload: &[u8],
        now_ms: u64,
    ) -> Result<(), SendError> {
        let Some(config) = self.config else {
            return Err(SendError::NotConfigured);
        };
        if ipv4::HEADER_BYTES + payload.len() > MAX_IP_DATAGRAM_BYTES {
            return Err(SendError::Oversize);
        }
        let Some(next_hop) = next_hop(&config, destination) else {
            return Err(SendError::NoRoute);
        };
        let mut datagram = Vec::new();
        if ipv4::encode(config.address, destination, protocol, payload, &mut datagram).is_err() {
            return Err(SendError::Oversize);
        }
        match self.arp.want(next_hop, now_ms) {
            Want::Resolved(mac) => self.transmit_to(mac, &datagram),
            Want::Started => {
                if self.waiting.len() == MAX_WAITING_FRAMES {
                    return Err(SendError::Busy);
                }
                self.waiting.push(Waiting {
                    next_hop,
                    destination,
                    datagram,
                });
                let request = arp::request(self.device.mac(), config.address, next_hop);
                self.counters.tx_frames += 1;
                self.device.transmit(&request);
            }
            Want::Coalesced => {
                if self.waiting.len() == MAX_WAITING_FRAMES {
                    return Err(SendError::Busy);
                }
                self.waiting.push(Waiting {
                    next_hop,
                    destination,
                    datagram,
                });
            }
            Want::Full => return Err(SendError::Busy),
        }
        Ok(())
    }

    fn flush_next_hop(&mut self, hop: [u8; IPV4_BYTES], now_ms: u64) {
        if !self.waiting.iter().any(|entry| entry.next_hop == hop) {
            return;
        }
        let Some(mac) = self.arp.lookup(hop, now_ms) else {
            return;
        };
        let mut parked = Vec::new();
        core::mem::swap(&mut parked, &mut self.waiting);
        for entry in parked {
            if entry.next_hop == hop {
                self.transmit_to(mac, &entry.datagram);
            } else {
                self.waiting.push(entry);
            }
        }
    }

    fn transmit_to(&mut self, mac: [u8; MAC_BYTES], datagram: &[u8]) {
        let mut frame = Vec::with_capacity(ethernet::HEADER_BYTES + datagram.len());
        frame.extend_from_slice(&mac);
        frame.extend_from_slice(&self.device.mac());
        frame.extend_from_slice(&ethernet::ETHERTYPE_IPV4.to_be_bytes());
        frame.extend_from_slice(datagram);
        self.counters.tx_frames += 1;
        self.device.transmit(&frame);
    }

    fn owns(&self, ip: [u8; IPV4_BYTES]) -> bool {
        self.config.is_some_and(|config| owns_address(&config, ip))
    }
}

fn owns_address(config: &Config, ip: [u8; IPV4_BYTES]) -> bool {
    ip == config.address || ip == [255, 255, 255, 255]
}

fn next_hop(config: &Config, destination: [u8; IPV4_BYTES]) -> Option<[u8; IPV4_BYTES]> {
    if same_bucket(config, destination) {
        Some(destination)
    } else {
        config.gateway
    }
}

fn prefix_mask(prefix_len: u8) -> u32 {
    if prefix_len >= 32 {
        u32::MAX
    } else if prefix_len == 0 {
        0
    } else {
        u32::MAX << (32 - prefix_len as u32)
    }
}

fn same_bucket(config: &Config, ip: [u8; IPV4_BYTES]) -> bool {
    let mask = prefix_mask(config.prefix_len);
    u32::from_be_bytes(config.address) & mask == u32::from_be_bytes(ip) & mask
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::MockDevice;
    use alloc::vec;

    const OWN_MAC: [u8; MAC_BYTES] = [0x02, 0x52, 0x4e, 0x45, 0x32, 0x07];
    const ROUTER_MAC: [u8; MAC_BYTES] = [0x52, 0x55, 0x0a, 0x00, 0x02, 0x00];
    const OWN: [u8; IPV4_BYTES] = [10, 0, 2, 15];
    const ROUTER: [u8; IPV4_BYTES] = [10, 0, 2, 2];
    const REMOTE: [u8; IPV4_BYTES] = [8, 8, 8, 8];

    fn configured() -> Stack<MockDevice> {
        let mut stack = Stack::new(MockDevice::new(OWN_MAC));
        stack.configure(Config {
            address: OWN,
            prefix_len: 24,
            gateway: Some(ROUTER),
        });
        stack
    }

    fn ipv4_frame(destination_mac: [u8; MAC_BYTES], source_mac: [u8; MAC_BYTES], datagram: &[u8]) -> Vec<u8> {
        let mut frame = Vec::new();
        frame.extend_from_slice(&destination_mac);
        frame.extend_from_slice(&source_mac);
        frame.extend_from_slice(&ethernet::ETHERTYPE_IPV4.to_be_bytes());
        frame.extend_from_slice(datagram);
        frame
    }

    fn echo_frame(source_mac: [u8; MAC_BYTES], source_ip: [u8; IPV4_BYTES], destination_ip: [u8; IPV4_BYTES], reply: bool, id: u16, sequence: u16, data: &[u8]) -> Vec<u8> {
        let kind = if reply { icmp::ECHO_REPLY } else { icmp::ECHO_REQUEST };
        let message = icmp::echo(kind, id, sequence, data);
        let mut datagram = Vec::new();
        ipv4::encode(source_ip, destination_ip, ipv4::PROTO_ICMP, &message, &mut datagram).expect("echo encodes");
        ipv4_frame(OWN_MAC, source_mac, &datagram)
    }

    fn expect_datagram(events: Vec<Event>, source: [u8; IPV4_BYTES], protocol: u8, payload: &[u8]) {
        assert_eq!(
            events,
            alloc::vec![Event::Datagram {
                source,
                protocol,
                payload: payload.to_vec(),
            }]
        );
    }

    #[test]
    fn arp_who_has_for_us_is_answered_verbatim() {
        let mut stack = configured();
        stack.on_frame(&arp::request(ROUTER_MAC, ROUTER, OWN), 0);
        let sent = stack.device().sent().to_vec();
        assert_eq!(sent.len(), 1);
        assert_eq!(
            sent[0].as_slice(),
            arp::reply(OWN_MAC, OWN, ROUTER_MAC, ROUTER).as_slice()
        );
        let counters = stack.counters();
        assert_eq!(counters.arp_answers, 1);
        assert_eq!(counters.tx_frames, 1);
    }

    #[test]
    fn arp_who_has_for_other_is_silent_but_learning_happens() {
        let mut stack = configured();
        stack.on_frame(&arp::request(ROUTER_MAC, ROUTER, [10, 0, 2, 99]), 0);
        assert_eq!(stack.device().sent_count(), 0);
        stack
            .send_ip(ROUTER, ipv4::PROTO_ICMP, b"x", 0)
            .expect("resolved from learned cache");
        let sent = stack.device().sent().to_vec();
        assert_eq!(sent.len(), 1);
        assert_eq!(&sent[0][0..6], &ROUTER_MAC);
        assert_eq!(stack.counters().tx_dropped, 0);
    }

    #[test]
    fn echo_request_is_answered_with_the_exact_transcript_frame() {
        let mut stack = configured();
        let events = stack.on_frame(&echo_frame(ROUTER_MAC, ROUTER, OWN, false, 0x1234, 7, b"rosx"), 0);
        let message = icmp::echo(icmp::ECHO_REPLY, 0x1234, 7, b"rosx");
        let mut datagram = Vec::new();
        ipv4::encode(OWN, ROUTER, ipv4::PROTO_ICMP, &message, &mut datagram).expect("reply encodes");
        let sent = stack.device().sent().to_vec();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0], ipv4_frame(ROUTER_MAC, OWN_MAC, &datagram));
        let counters = stack.counters();
        assert_eq!(counters.echo_answers, 1);
        expect_datagram(
            events,
            ROUTER,
            ipv4::PROTO_ICMP,
            &icmp::echo(icmp::ECHO_REQUEST, 0x1234, 7, b"rosx"),
        );
    }

    #[test]
    fn echo_reply_is_delivered_as_event_without_answer() {
        let mut stack = configured();
        stack.arp.learn(ROUTER, ROUTER_MAC, 0);
        let events = stack.on_frame(&echo_frame(ROUTER_MAC, ROUTER, OWN, true, 0x1234, 7, b"pong"), 0);
        expect_datagram(events, ROUTER, ipv4::PROTO_ICMP, &icmp::echo(icmp::ECHO_REPLY, 0x1234, 7, b"pong"));
        assert_eq!(stack.device().sent_count(), 0);
        assert_eq!(stack.counters().echo_answers, 0);
    }

    #[test]
    fn foreign_destination_is_dropped() {
        let mut stack = configured();
        let events = stack.on_frame(&echo_frame(ROUTER_MAC, ROUTER, [10, 0, 2, 99], false, 1, 1, b"x"), 0);
        assert!(events.is_empty());
        assert_eq!(stack.device().sent_count(), 0);
        assert_eq!(stack.counters().rx_dropped, 1);
    }

    #[test]
    fn foreign_destination_mac_is_dropped() {
        let mut stack = configured();
        let mut frame = echo_frame(ROUTER_MAC, ROUTER, OWN, false, 1, 1, b"x");
        frame[0..6].copy_from_slice(&[0x02, 0xff, 0xff, 0xff, 0xff, 0xff]);
        assert!(stack.on_frame(&frame, 0).is_empty());
        assert_eq!(stack.counters().rx_dropped, 1);
    }

    #[test]
    fn corrupted_ipv4_header_is_dropped() {
        let mut stack = configured();
        let mut frame = echo_frame(ROUTER_MAC, ROUTER, OWN, false, 1, 1, b"x");
        let corrupt_at = ethernet::HEADER_BYTES + 8;
        frame[corrupt_at] ^= 0x01;
        assert!(stack.on_frame(&frame, 0).is_empty());
        assert_eq!(stack.counters().rx_dropped, 1);
        assert_eq!(stack.device().sent_count(), 0);
    }

    #[test]
    fn fragments_are_dropped() {
        let mut stack = configured();
        let mut frame = echo_frame(ROUTER_MAC, ROUTER, OWN, false, 1, 1, b"x");
        frame[ethernet::HEADER_BYTES + 6..ethernet::HEADER_BYTES + 8].copy_from_slice(&[0x20, 0x00]);
        let header_len = (frame[ethernet::HEADER_BYTES] & 0x0F) as usize * 4;
        let checksum_start = ethernet::HEADER_BYTES + 10;
        frame[checksum_start..checksum_start + 2].copy_from_slice(&[0, 0]);
        let sum = ipv4::checksum(&frame[ethernet::HEADER_BYTES..ethernet::HEADER_BYTES + header_len]);
        frame[checksum_start..checksum_start + 2].copy_from_slice(&sum.to_be_bytes());
        assert!(stack.on_frame(&frame, 0).is_empty());
        assert_eq!(stack.counters().rx_dropped, 1);
    }

    #[test]
    fn foreign_ethertype_is_dropped() {
        let mut stack = configured();
        let mut frame = vec![0xffu8; 6];
        frame.extend_from_slice(&ROUTER_MAC);
        frame.extend_from_slice(&0x88ccu16.to_be_bytes());
        frame.resize(60, 0);
        assert!(stack.on_frame(&frame, 0).is_empty());
        assert_eq!(stack.counters().rx_dropped, 1);
    }

    #[test]
    fn unconfigured_stack_refuses_sends_and_stays_silent() {
        let mut stack = Stack::new(MockDevice::new(OWN_MAC));
        assert_eq!(
            stack.send_ip(ROUTER, ipv4::PROTO_ICMP, b"x", 0),
            Err(SendError::NotConfigured)
        );
        stack.on_frame(&arp::request(ROUTER_MAC, ROUTER, OWN), 0);
        let events = stack.on_frame(&echo_frame(ROUTER_MAC, ROUTER, OWN, false, 1, 1, b"x"), 0);
        assert!(events.is_empty());
        assert_eq!(stack.device().sent_count(), 0);
    }

    #[test]
    fn resolve_then_echo_request_full_transcript() {
        let mut stack = configured();
        let message = icmp::echo(icmp::ECHO_REQUEST, 0xabcd, 1, b"ping");
        stack
            .send_ip(ROUTER, ipv4::PROTO_ICMP, &message, 0)
            .expect("resolve starts");
        let sent = stack.device().sent().to_vec();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].as_slice(), arp::request(OWN_MAC, OWN, ROUTER).as_slice());
        let after_resolve = stack.device().sent_count();
        stack.on_frame(&arp::reply(ROUTER_MAC, ROUTER, OWN_MAC, OWN), 10);
        let sent = stack.device().sent()[after_resolve..].to_vec();
        assert_eq!(sent.len(), 1);
        assert_eq!(&sent[0][0..6], &ROUTER_MAC);
        let header = ethernet::decode(&sent[0]).expect("frame decodes");
        let packet = ipv4::decode(header.payload).expect("datagram decodes");
        assert_eq!(packet.source, OWN);
        assert_eq!(packet.destination, ROUTER);
        match icmp::decode(packet.payload).expect("icmp decodes") {
            Icmp::Echo {
                reply,
                id,
                sequence,
                data,
            } => {
                assert!(!reply);
                assert_eq!(id, 0xabcd);
                assert_eq!(sequence, 1);
                assert_eq!(data, b"ping");
            }
            Icmp::Other { kind, .. } => panic!("echo request decoded as kind {kind}"),
        }
    }

    #[test]
    fn gateway_is_the_next_hop_for_remote_destinations() {
        let mut stack = configured();
        stack
            .send_ip(REMOTE, ipv4::PROTO_ICMP, b"probe", 0)
            .expect("resolve starts");
        let sent = stack.device().sent().to_vec();
        let arp_packet = arp::decode(
            &ethernet::decode(&sent[0]).expect("frame decodes").payload.to_vec(),
        )
        .expect("arp decodes");
        assert_eq!(arp_packet.target_ip, ROUTER);
        let after_resolve = stack.device().sent_count();
        stack.on_frame(&arp::reply(ROUTER_MAC, ROUTER, OWN_MAC, OWN), 10);
        let sent = stack.device().sent()[after_resolve..].to_vec();
        assert_eq!(&sent[0][0..6], &ROUTER_MAC);
        let packet = ipv4::decode(ethernet::decode(&sent[0]).expect("frame").payload).expect("datagram");
        assert_eq!(packet.destination, REMOTE);
    }

    #[test]
    fn remote_without_gateway_is_no_route() {
        let mut stack = configured();
        stack.configure(Config {
            address: OWN,
            prefix_len: 24,
            gateway: None,
        });
        assert_eq!(
            stack.send_ip(REMOTE, ipv4::PROTO_ICMP, b"x", 0),
            Err(SendError::NoRoute)
        );
        assert_eq!(stack.device().sent_count(), 0);
        stack
            .send_ip(ROUTER, ipv4::PROTO_ICMP, b"x", 0)
            .expect("same bucket needs no gateway");
    }

    #[test]
    fn retries_three_times_then_reports_unreachable() {
        let mut stack = configured();
        stack
            .send_ip(ROUTER, ipv4::PROTO_ICMP, b"help", 0)
            .expect("resolve starts");
        assert!(stack.tick(1_000).is_empty());
        assert!(stack.tick(2_000).is_empty());
        let events = stack.tick(3_000);
        assert_eq!(events, alloc::vec![Event::Unreachable { destination: ROUTER }]);
        assert_eq!(stack.device().sent_count(), 3);
        let counters = stack.counters();
        assert_eq!(counters.tx_dropped, 1);
        assert_eq!(stack.tick(10_000).len(), 0);
    }

    #[test]
    fn coalesced_waiters_share_one_who_has() {
        let mut stack = configured();
        stack
            .send_ip(ROUTER, ipv4::PROTO_ICMP, b"one", 0)
            .expect("first starts resolve");
        stack
            .send_ip(ROUTER, ipv4::PROTO_ICMP, b"two", 10)
            .expect("second coalesces");
        assert_eq!(stack.device().sent_count(), 1);
        let after_resolve = stack.device().sent_count();
        stack.on_frame(&arp::reply(ROUTER_MAC, ROUTER, OWN_MAC, OWN), 20);
        let sent = stack.device().sent()[after_resolve..].to_vec();
        assert_eq!(sent.len(), 2);
        assert!(sent.iter().all(|frame| &frame[0..6] == &ROUTER_MAC));
    }

    #[test]
    fn oversize_payloads_are_refused_at_the_mtu_edge() {
        let mut stack = configured();
        stack.arp.learn(ROUTER, ROUTER_MAC, 0);
        let fits = vec![0u8; MAX_IP_DATAGRAM_BYTES - ipv4::HEADER_BYTES];
        stack
            .send_ip(ROUTER, ipv4::PROTO_ICMP, &fits, 0)
            .expect("1500-byte datagram fits");
        let oversized = vec![0u8; MAX_IP_DATAGRAM_BYTES - ipv4::HEADER_BYTES + 1];
        assert_eq!(
            stack.send_ip(ROUTER, ipv4::PROTO_ICMP, &oversized, 0),
            Err(SendError::Oversize)
        );
        assert_eq!(stack.device().sent_count(), 1);
    }

    #[test]
    fn waiting_queue_is_bounded_and_busy() {
        let mut stack = configured();
        for index in 0..MAX_WAITING_FRAMES {
            let target = [10, 0, 2, 100 + index as u8];
            stack
                .send_ip(target, ipv4::PROTO_ICMP, b"q", 0)
                .expect("queue has room");
        }
        assert_eq!(
            stack.send_ip([10, 0, 2, 200], ipv4::PROTO_ICMP, b"q", 0),
            Err(SendError::Busy)
        );
        assert_eq!(stack.device().sent_count(), MAX_WAITING_FRAMES);
    }

    #[test]
    fn limited_broadcast_echo_request_is_delivered_without_answer() {
        let mut stack = configured();
        let events = stack.on_frame(
            &echo_frame(ROUTER_MAC, ROUTER, [255, 255, 255, 255], false, 9, 9, b"shout"),
            0,
        );
        expect_datagram(events, ROUTER, ipv4::PROTO_ICMP, &icmp::echo(icmp::ECHO_REQUEST, 9, 9, b"shout"));
        assert_eq!(stack.device().sent_count(), 0);
    }

    #[test]
    fn non_icmp_datagram_is_delivered_raw() {
        let mut stack = configured();
        let mut datagram = Vec::new();
        ipv4::encode(ROUTER, OWN, 6, b"tcp-bytes", &mut datagram).expect("encodes");
        let events = stack.on_frame(&ipv4_frame(OWN_MAC, ROUTER_MAC, &datagram), 0);
        assert_eq!(
            events,
            alloc::vec![Event::Datagram {
                source: ROUTER,
                protocol: 6,
                payload: b"tcp-bytes".to_vec(),
            }]
        );
        assert_eq!(stack.device().sent_count(), 0);
    }

    #[test]
    fn prefix_masks_split_buckets() {
        let config = Config {
            address: [10, 0, 2, 15],
            prefix_len: 24,
            gateway: None,
        };
        assert!(same_bucket(&config, [10, 0, 2, 200]));
        assert!(!same_bucket(&config, [10, 0, 3, 1]));
        let wide = Config {
            address: [10, 0, 2, 15],
            prefix_len: 8,
            gateway: None,
        };
        assert!(same_bucket(&wide, [10, 9, 9, 9]));
        assert!(same_bucket(&Config {
            address: [10, 0, 2, 15],
            prefix_len: 0,
            gateway: None,
        }, [1, 1, 1, 1]));
        assert!(same_bucket(&Config {
            address: [10, 0, 2, 15],
            prefix_len: 32,
            gateway: None,
        }, [10, 0, 2, 15]));
        assert!(!same_bucket(&Config {
            address: [10, 0, 2, 15],
            prefix_len: 32,
            gateway: None,
        }, [10, 0, 2, 16]));
    }
}
