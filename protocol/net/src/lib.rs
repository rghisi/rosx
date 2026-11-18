#![cfg_attr(not(test), no_std)]

extern crate alloc;

pub mod arp;
pub mod client;
pub mod ethernet;
pub mod icmp;
pub mod ipv4;
pub mod wire;

pub(crate) fn u16_at(bytes: &[u8], at: usize) -> u16 {
    u16::from_be_bytes([bytes[at], bytes[at + 1]])
}
