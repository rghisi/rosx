#![no_std]

extern crate alloc;

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use net_protocol::client::Ne2k;
use net_protocol::wire::{MAC_BYTES, Stats};
use usrlib::println;

const ETHERNET_MIN_BYTES: usize = 60;
const ETHERNET_MAX_BYTES: usize = 1514;

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

fn report_stats(stats: &Stats) {
    println!(
        "ne2k tx={} tx-errors={} rx={} rx-errors={} overruns={} pushes={} push-drops={}",
        stats.tx_frames,
        stats.tx_errors,
        stats.rx_frames,
        stats.rx_errors,
        stats.rx_overruns,
        stats.watch_pushes,
        stats.watch_drops
    );
}

fn report_frame(frame: &[u8]) {
    let shown = frame.len().min(16);
    let mut bytes = String::new();
    for byte in &frame[..shown] {
        bytes.push_str(&format!("{byte:02x} "));
    }
    println!("ne2k frame {} bytes: {bytes}", frame.len());
}

fn parse_hex_frame(text: &str) -> Option<Vec<u8>> {
    let digits = text.replace(':', "");
    if digits.is_empty() || digits.len() % 2 != 0 || digits.len() > ETHERNET_MAX_BYTES * 2 {
        return None;
    }
    let mut frame = Vec::with_capacity(digits.len() / 2);
    let bytes = digits.as_bytes();
    for pair in bytes.chunks(2) {
        let byte = u8::from_str_radix(core::str::from_utf8(pair).ok()?, 16).ok()?;
        frame.push(byte);
    }
    if frame.len() < ETHERNET_MIN_BYTES {
        frame.resize(ETHERNET_MIN_BYTES, 0);
    }
    Some(frame)
}

fn main() {
    let arguments = usrlib::args();
    let requested: &[&str] = if arguments.is_empty() {
        &[]
    } else {
        &arguments[1..]
    };
    let Ok(nic) = Ne2k::connect() else {
        println!("ne2k no service");
        return;
    };
    match requested.first().copied() {
        None | Some("mac") => match nic.mac() {
            Ok(mac) => println!("ne2k mac {}", format_mac(mac)),
            Err(_) => println!("ne2k mac failed"),
        },
        Some("stats") => match nic.stats() {
            Ok(stats) => report_stats(&stats),
            Err(_) => println!("ne2k stats failed"),
        },
        Some("poll") => match nic.poll() {
            Ok(Some(frame)) => report_frame(&frame),
            Ok(None) => println!("ne2k no frame"),
            Err(_) => println!("ne2k poll failed"),
        },
        Some("send") => {
            let Some(hex) = requested.get(1).copied() else {
                println!("ne2k usage: ne2k send <hex-digits>");
                return;
            };
            let Some(frame) = parse_hex_frame(hex) else {
                println!("ne2k send needs up to {} hex bytes", ETHERNET_MAX_BYTES);
                return;
            };
            match nic.send(&frame) {
                Ok(()) => println!("ne2k sent {} bytes", frame.len()),
                Err(_) => println!("ne2k send failed"),
            }
        }
        Some("watch") => {
            let count = requested
                .get(1)
                .and_then(|text| text.parse::<u32>().ok())
                .unwrap_or(1);
            if nic.watch().is_err() {
                println!("ne2k watch subscribe failed");
                return;
            }
            for _ in 0..count {
                match nic.receive_frame() {
                    Ok(frame) => report_frame(&frame),
                    Err(err) => {
                        println!("ne2k watch receive failed: {err:?}");
                        break;
                    }
                }
            }
        }
        Some(subcommand) => println!("ne2k unknown subcommand: {subcommand}"),
    }
    nic.disconnect();
}
