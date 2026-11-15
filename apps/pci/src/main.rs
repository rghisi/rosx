#![no_std]

extern crate alloc;

use pci_protocol::client::Pci;
use pci_protocol::wire::Slot;
use usrlib::println;

fn parse_hex(value: &str) -> Option<u32> {
    let digits = value.strip_prefix("0x").or(value.strip_prefix("0X")).unwrap_or(value);
    if digits.is_empty() || digits.len() > 8 {
        return None;
    }
    u32::from_str_radix(digits, 16).ok()
}

fn report(slot: &Slot) {
    if let Some(line) = slot.location() {
        println!("pci {line}");
    }
}

fn main() {
    let arguments = usrlib::args();
    let requested: &[&str] = if arguments.is_empty() {
        &[]
    } else {
        &arguments[1..]
    };
    let Ok(pci) = Pci::connect() else {
        println!("pci no service");
        return;
    };
    match requested.first().copied() {
        None => match pci.list(0) {
            Ok(slots) => {
                for slot in &slots {
                    report(slot);
                }
                println!("pci bus 0: {} functions", slots.len());
            }
            Err(_) => println!("pci list failed"),
        },
        Some("find") => {
            let Some(vendor) = requested.get(1).and_then(|text| parse_hex(text)) else {
                println!("pci usage: pci find <vendor-id> <device-id>");
                return;
            };
            let Some(device) = requested.get(2).and_then(|text| parse_hex(text)) else {
                println!("pci usage: pci find <vendor-id> <device-id>");
                return;
            };
            match pci.find(vendor as u16, device as u16) {
                Ok(Some(slot)) => report(&slot),
                Ok(None) => println!("pci absent {:04x}:{:04x}", vendor as u16, device as u16),
                Err(_) => println!("pci find failed"),
            }
        }
        Some(subcommand) => println!("pci unknown subcommand: {subcommand}"),
    }
    pci.disconnect();
}
