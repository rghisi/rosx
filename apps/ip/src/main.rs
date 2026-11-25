#![no_std]

extern crate alloc;

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use ip_protocol::client::{Ip, IpEvent};
use ip_protocol::wire::{self, IPV4_BYTES};
use net_protocol::icmp::{self, Icmp};
use usrlib::syscall::Syscall;
use usrlib::println;

const PING_TIMEOUT_MS: u64 = 2_000;
const PING_POLL_MS: u64 = 20;
const PING_GAP_MS: u64 = 400;
const DEFAULT_PING_COUNT: u32 = 3;
const DEFAULT_PING_SIZE: usize = 56;
const MAX_PING_SIZE: usize = 1_472;
const PING_PATTERN: &[u8] = b"RosxPing";

fn format_mac(mac: [u8; 6]) -> String {
    let mut rendered = String::new();
    for (index, byte) in mac.iter().enumerate() {
        if index > 0 {
            rendered.push(':');
        }
        rendered.push_str(&format!("{byte:02x}"));
    }
    rendered
}

fn parse_prefix(text: &str) -> Option<u8> {
    let value: u8 = text.parse().ok()?;
    if value <= 32 {
        Some(value)
    } else {
        None
    }
}

struct AddressSpec {
    address: [u8; IPV4_BYTES],
    prefix_len: u8,
}

fn parse_address_spec(text: &str) -> Option<AddressSpec> {
    match text.split_once('/') {
        Some((address, prefix)) => Some(AddressSpec {
            address: wire::parse_address(address)?,
            prefix_len: parse_prefix(prefix)?,
        }),
        None => Some(AddressSpec {
            address: wire::parse_address(text)?,
            prefix_len: 24,
        }),
    }
}

fn show_status(ip: &mut Ip) {
    match ip.status() {
        Ok(status) => {
            println!(
                "ip link={} configured={} addr={}/{} gw={} mac={}",
                if status.link { "up" } else { "down" },
                status.configured,
                wire::format_address(status.address),
                status.prefix_len,
                status
                    .gateway
                    .map(wire::format_address)
                    .unwrap_or_else(|| "none".into()),
                format_mac(status.mac)
            );
            println!(
                "ip counters rx={} tx={} rx-dropped={} tx-dropped={} arp-answers={} echo-answers={}",
                status.counters.rx_frames,
                status.counters.tx_frames,
                status.counters.rx_dropped,
                status.counters.tx_dropped,
                status.counters.arp_answers,
                status.counters.echo_answers
            );
        }
        Err(err) => println!("ip status failed: {err:?}"),
    }
}

fn show_neighbours(ip: &mut Ip) {
    match ip.neighbours() {
        Ok(neighbours) => {
            if neighbours.is_empty() {
                println!("ip neigh empty");
            }
            let now = Syscall::now();
            for neighbour in neighbours {
                println!(
                    "{} {} age={}ms",
                    wire::format_address(neighbour.address),
                    format_mac(neighbour.mac),
                    now.saturating_sub(neighbour.updated_ms)
                );
            }
        }
        Err(err) => println!("ip neigh failed: {err:?}"),
    }
}

fn apply_config(ip: &mut Ip, spec: &AddressSpec, gateway: Option<[u8; IPV4_BYTES]>) {
    match ip.configure(spec.address, spec.prefix_len, gateway) {
        Ok(()) => println!(
            "ip configured {}/{} gw {}",
            wire::format_address(spec.address),
            spec.prefix_len,
            gateway
                .map(wire::format_address)
                .unwrap_or_else(|| "none".into())
        ),
        Err(err) => println!("ip config failed: {err:?}"),
    }
}

fn wait_for_reply(
    ip: &mut Ip,
    destination: [u8; IPV4_BYTES],
    id: u16,
    sequence: u16,
) -> Option<u64> {
    let started = Syscall::now();
    loop {
        let Ok(future) = ip.receive_future() else {
            return None;
        };
        while !Syscall::is_future_completed(future) {
            if Syscall::now().saturating_sub(started) >= PING_TIMEOUT_MS {
                return None;
            }
            Syscall::sleep(PING_POLL_MS);
        }
        let elapsed = Syscall::now().saturating_sub(started);
        if elapsed >= PING_TIMEOUT_MS {
            return None;
        }
        match ip.consume_receive(future) {
            Ok(IpEvent::Datagram {
                source,
                protocol,
                payload,
            }) => {
                if source != destination || protocol != net_protocol::ipv4::PROTO_ICMP {
                    continue;
                }
                if let Ok(Icmp::Echo {
                    reply: true,
                    id: reply_id,
                    sequence: reply_sequence,
                    ..
                }) = icmp::decode(&payload)
                {
                    if reply_id == id && reply_sequence == sequence {
                        return Some(elapsed);
                    }
                }
            }
            Ok(IpEvent::Unreachable { destination: unreachable, .. }) => {
                if unreachable == destination {
                    println!("From {}: Destination Host Unreachable", wire::format_address(destination));
                    return None;
                }
            }
            Err(_) => return None,
        }
    }
}

fn run_ping(ip: &mut Ip, destination: [u8; IPV4_BYTES], count: u32, size: usize) {
    if size > MAX_PING_SIZE {
        println!("ip ping size must be <= {MAX_PING_SIZE}");
        return;
    }
    if ip.subscribe(icmp::ECHO_REPLY).is_err() {
        println!("ip ping subscribe failed");
        return;
    }
    let id = (Syscall::now() & 0xffff) as u16;
    let mut sent = 0u32;
    let mut received = 0u32;
    let destination_text = wire::format_address(destination);
    for sequence in 1..=count as u16 {
        let mut payload = Vec::with_capacity(size);
        for index in 0..size {
            payload.push(PING_PATTERN[index % PING_PATTERN.len()]);
        }
        let message = icmp::echo(icmp::ECHO_REQUEST, id, sequence, &payload);
        if ip.send_ip(destination, net_protocol::ipv4::PROTO_ICMP, &message).is_err() {
            println!("ip ping send failed");
            break;
        }
        sent += 1;
        match wait_for_reply(ip, destination, id, sequence) {
            Some(rtt) => {
                received += 1;
                println!(
                    "{} bytes from {destination_text}: icmp_seq={sequence} time={rtt} ms",
                    message.len()
                );
            }
            None => println!("icmp_seq={sequence} no reply"),
        }
        if sequence < count as u16 {
            Syscall::sleep(PING_GAP_MS);
        }
    }
    let lost = sent - received;
    let loss_percent = if sent == 0 {
        100
    } else {
        lost * 100 / sent
    };
    println!(
        "ping: {sent} transmitted, {received} received, {loss_percent}% loss"
    );
}

fn main() {
    let arguments = usrlib::args();
    let requested: &[&str] = if arguments.is_empty() {
        &[]
    } else {
        &arguments[1..]
    };
    let Ok(mut ip) = Ip::connect() else {
        println!("ip no service");
        return;
    };
    match requested.first().copied() {
        None | Some("status") => show_status(&mut ip),
        Some("config") => {
            let Some(spec_text) = requested.get(1).copied() else {
                println!("usage: ip config <addr>[/len] [gw <addr>]");
                return;
            };
            let Some(spec) = parse_address_spec(spec_text) else {
                println!("ip config bad address: {spec_text}");
                return;
            };
            let gateway = match requested.get(2).copied() {
                None => None,
                Some("gw") => match requested.get(3).copied().map(wire::parse_address) {
                    Some(Some(gateway)) => Some(gateway),
                    _ => {
                        println!("ip config bad gateway");
                        return;
                    }
                },
                Some(other) => {
                    println!("ip config unknown option: {other}");
                    return;
                }
            };
            apply_config(&mut ip, &spec, gateway);
        }
        Some("neigh") => show_neighbours(&mut ip),
        Some("ping") => {
            let Some(target_text) = requested.get(1).copied() else {
                println!("usage: ip ping <addr> [count] [size]");
                return;
            };
            let Some(destination) = wire::parse_address(target_text) else {
                println!("ip ping bad address: {target_text}");
                return;
            };
            let count = requested
                .get(2)
                .and_then(|text| text.parse::<u32>().ok())
                .filter(|count| *count >= 1)
                .unwrap_or(DEFAULT_PING_COUNT);
            let size = requested
                .get(3)
                .and_then(|text| text.parse::<usize>().ok())
                .unwrap_or(DEFAULT_PING_SIZE);
            run_ping(&mut ip, destination, count, size);
        }
        Some(subcommand) => println!("ip unknown subcommand: {subcommand}"),
    }
    ip.disconnect();
}
