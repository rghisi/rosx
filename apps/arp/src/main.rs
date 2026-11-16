#![no_std]

extern crate alloc;

use alloc::format;
use alloc::string::{String, ToString};
use arp::arp::{self as wire, ArpError, ArpPacket, IPV4_BYTES, MAC_BYTES};
use net_protocol::client::Ne2k;
use net_protocol::wire as net_wire;
use pci_protocol::wire::SERVICE_NAME as PCI_SERVICE_NAME;
use usrlib::syscall::Syscall;
use usrlib::println;
use vfs_protocol::client::Vfs;

const LAUNCH_ATTEMPTS: usize = 100;
const LAUNCH_RETRY_DELAY_MS: u64 = 100;
const DEFAULT_WATCH_SECONDS: u64 = 10;
const WHO_HAS_TIMEOUT_MS: u64 = 5_000;
const POLL_INTERVAL_MS: u64 = 10;

fn format_mac(mac: [u8; MAC_BYTES]) -> String {
    let mut rendered = String::new();
    for (index, byte) in mac.iter().enumerate() {
        if index > 0 {
            rendered.push(':');
        }
        rendered.push_str(&format!("{byte:02x}"));
    }
    rendered
}

fn format_ipv4(address: [u8; 4]) -> String {
    format!(
        "{}.{}.{}.{}",
        address[0], address[1], address[2], address[3]
    )
}

fn describe_error(error: ArpError) -> &'static str {
    match error {
        ArpError::TooShort => "too short",
        ArpError::HardwareFormat => "hardware format",
        ArpError::ProtocolFormat => "protocol format",
        ArpError::AddressSize => "address size",
    }
}

fn describe_packet(packet: &ArpPacket) -> String {
    let sender = format!("{} ({})", format_ipv4(packet.sender_ip), format_mac(packet.sender_mac));
    let target = format!("{} ({})", format_ipv4(packet.target_ip), format_mac(packet.target_mac));
    match packet.operation {
        wire::Operation::Request => format!(
            "arp request: who-has {} tell {}",
            format_ipv4(packet.target_ip),
            format_ipv4(packet.sender_ip)
        ),
        wire::Operation::Reply => format!("arp reply: {} is at {}", sender, format_mac(packet.sender_mac)),
        wire::Operation::Other(code) => format!("arp op {code}: {sender} -> {target}"),
    }
}

struct WatchTally {
    arp_frames: u32,
    malformed: u32,
    other: u32,
}

fn watch(nic: &Ne2k, seconds: u64) {
    let deadline = Syscall::now() + seconds * 1000;
    let mut tally = WatchTally { arp_frames: 0, malformed: 0, other: 0 };
    loop {
        match nic.poll() {
            Ok(Some(frame)) => match wire::decode_ethernet(&frame) {
                Err(_) => {
                    tally.malformed += 1;
                    println!("arp runt: {} bytes", frame.len());
                }
                Ok(header) if header.ethertype != wire::ETHERTYPE_ARP => {
                    tally.other += 1;
                    println!(
                        "arp skip: ethertype {:04x} from {}",
                        header.ethertype,
                        format_mac(header.source)
                    );
                }
                Ok(header) => match wire::decode_arp(header.payload) {
                    Ok(packet) => {
                        tally.arp_frames += 1;
                        println!("{}", describe_packet(&packet));
                    }
                    Err(error) => {
                        tally.malformed += 1;
                        println!(
                            "arp malformed: {} ({} bytes)",
                            describe_error(error),
                            frame.len()
                        );
                    }
                },
            },
            Ok(None) => {
                if Syscall::now() >= deadline {
                    break;
                }
                Syscall::sleep(POLL_INTERVAL_MS);
            }
            Err(_) => {
                println!("arp poll failed");
                break;
            }
        }
    }
    println!(
        "arp watch done: {} arp, {} malformed, {} other",
        tally.arp_frames, tally.malformed, tally.other
    );
}

fn service_is_live(binding: &str) -> bool {
    Syscall::ipc_connect(binding).is_ok()
}

fn is_arp_reply_from(packet_source: &wire::ArpPacket, target: [u8; IPV4_BYTES]) -> bool {
    packet_source.operation == wire::Operation::Reply && packet_source.sender_ip == target
}

fn who_has(nic: &Ne2k, source_ip: [u8; IPV4_BYTES], target_ip: [u8; IPV4_BYTES]) {
    let Ok(mac) = nic.mac() else {
        println!("arp mac failed");
        return;
    };
    if nic.send(&wire::request(mac, source_ip, target_ip)).is_err() {
        println!("arp send failed");
        return;
    }
    println!(
        "arp who-has {} tell {}",
        format_ipv4(target_ip),
        format_ipv4(source_ip)
    );
    let deadline = Syscall::now() + WHO_HAS_TIMEOUT_MS;
    loop {
        match nic.poll() {
            Ok(Some(frame)) => {
                let answer = match wire::decode_ethernet(&frame) {
                    Ok(header) if header.ethertype == wire::ETHERTYPE_ARP => wire::decode_arp(header.payload)
                        .ok()
                        .filter(|packet| is_arp_reply_from(packet, target_ip)),
                    _ => None,
                };
                if let Some(packet) = answer {
                    println!(
                        "arp {} is at {}",
                        format_ipv4(target_ip),
                        format_mac(packet.sender_mac)
                    );
                    return;
                }
            }
            Ok(None) => {
                if Syscall::now() >= deadline {
                    println!("arp no reply from {}", format_ipv4(target_ip));
                    return;
                }
                Syscall::sleep(POLL_INTERVAL_MS);
            }
            Err(_) => {
                println!("arp poll failed");
                return;
            }
        }
    }
}

fn launch_detached(vfs: &Vfs, program: &str) {
    let Ok(image) = vfs.read_whole(&format!("/{program}")) else {
        return;
    };
    let argv = [program.to_string()];
    let _completion = Syscall::load_with_args(&image, &argv);
}

fn ensure_service(vfs: &Vfs, program: &str, binding: &str) {
    if service_is_live(binding) {
        return;
    }
    launch_detached(vfs, program);
    for _ in 0..LAUNCH_ATTEMPTS {
        if service_is_live(binding) {
            return;
        }
        Syscall::sleep(LAUNCH_RETRY_DELAY_MS);
    }
}

fn ensure_network_stack() {
    let Ok(vfs) = Vfs::connect() else {
        return;
    };
    ensure_service(&vfs, "pcid", PCI_SERVICE_NAME);
    ensure_service(&vfs, "ne2kd", net_wire::SERVICE_NAME);
    vfs.disconnect();
}

fn parse_seconds(text: &str) -> Option<u64> {
    text.parse::<u64>().ok()
}

fn parse_ipv4(text: &str) -> Option<[u8; IPV4_BYTES]> {
    let mut octets = [0u8; IPV4_BYTES];
    let mut parts = text.split('.');
    for slot in octets.iter_mut() {
        *slot = parts.next()?.parse().ok()?;
    }
    if parts.next().is_some() {
        return None;
    }
    Some(octets)
}

fn main() {
    let arguments = usrlib::args();
    let requested: &[&str] = if arguments.is_empty() {
        &[]
    } else {
        &arguments[1..]
    };
    if !service_is_live(net_wire::SERVICE_NAME) {
        ensure_network_stack();
    }
    let Ok(nic) = Ne2k::connect() else {
        println!("arp no service");
        return;
    };
    match requested.first().copied() {
        Some("watch") => {
            let seconds = requested
                .get(1)
                .and_then(|text| parse_seconds(text))
                .unwrap_or(DEFAULT_WATCH_SECONDS);
            watch(&nic, seconds);
        }
        Some("who-has") => {
            let Some(target_text) = requested.get(1).copied() else {
                println!("arp usage: arp who-has <tip> [from <sip>]");
                return;
            };
            let Some(target_ip) = parse_ipv4(target_text) else {
                println!("arp invalid address: {target_text}");
                return;
            };
            let mut source_ip = [0u8; IPV4_BYTES];
            match (requested.get(2).copied(), requested.get(3).copied()) {
                (None, _) => {}
                (Some("from"), Some(source_text)) => {
                    let Some(parsed) = parse_ipv4(source_text) else {
                        println!("arp invalid address: {source_text}");
                        return;
                    };
                    source_ip = parsed;
                }
                (Some(unexpected), _) => {
                    println!("arp unexpected argument: {unexpected}");
                    return;
                }
            }
            who_has(&nic, source_ip, target_ip);
        }
        Some(subcommand) => println!("arp unknown subcommand: {subcommand}"),
        None => println!("arp usage: arp watch [secs] | arp who-has <tip> [from <sip>]"),
    }
    nic.disconnect();
}
