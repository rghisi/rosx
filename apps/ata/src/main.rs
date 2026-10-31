#![no_std]

extern crate alloc;

use alloc::vec::Vec;
use ata_protocol::client::{Ata, DriveIdentity};
use usrlib::println;

const MBR_BYTES: usize = 512;
const MBR_PARTITION_FIRST: usize = 446;
const MBR_PARTITION_BYTES: usize = 16;
const MBR_PARTITION_COUNT: usize = 4;
const MBR_SIGNATURE_OFFSET: usize = 510;
const MBR_SIGNATURE_FIRST: u8 = 0x55;
const MBR_SIGNATURE_SECOND: u8 = 0xAA;

fn main() {
    let arguments = usrlib::args();
    let requested: &[&str] = if arguments.is_empty() {
        &[]
    } else {
        &arguments[1..]
    };
    let Ok(ata) = Ata::connect() else {
        println!("ata no service");
        return;
    };
    match requested.first().copied() {
        None => show_drive_and_mbr(&ata),
        Some("read") => read_subcommand(&ata, &requested[1..]),
        Some(subcommand) => println!("ata unknown subcommand: {subcommand}"),
    }
    ata.disconnect();
}

fn show_drive_and_mbr(ata: &Ata) {
    match ata.identify(0) {
        Ok(identity) => report_identity(&identity),
        Err(error) => {
            println!("ata identify failed: {error:?}");
            return;
        }
    }
    match ata.read(0, 0, 1) {
        Ok(mbr) => report_mbr(&mbr),
        Err(error) => println!("ata read failed: {error:?}"),
    }
}

fn report_identity(identity: &DriveIdentity) {
    println!(
        "ata model={} firmware={} sectors={}",
        identity.model, identity.firmware_rev, identity.sectors
    );
}

fn report_mbr(mbr: &[u8]) {
    if mbr.len() < MBR_BYTES {
        println!("mbr short read bytes={}", mbr.len());
        return;
    }
    let first = mbr[MBR_SIGNATURE_OFFSET];
    let second = mbr[MBR_SIGNATURE_OFFSET + 1];
    if first == MBR_SIGNATURE_FIRST && second == MBR_SIGNATURE_SECOND {
        println!("mbr signature 55AA ok");
    } else {
        println!("mbr signature {first:02X}{second:02X} bad");
    }
    for index in 0..MBR_PARTITION_COUNT {
        report_partition(index, &mbr[MBR_PARTITION_FIRST + index * MBR_PARTITION_BYTES..]);
    }
}

fn report_partition(index: usize, entry: &[u8]) {
    let lba = u32::from_le_bytes([entry[8], entry[9], entry[10], entry[11]]);
    let sectors = u32::from_le_bytes([entry[12], entry[13], entry[14], entry[15]]);
    println!(
        "mbr part{index} boot={:02X} type={:02X} lba={lba} sectors={sectors}",
        entry[0], entry[4]
    );
}

fn read_subcommand(ata: &Ata, arguments: &[&str]) {
    let [drive, lba, sectors] = arguments else {
        println!("usage: ata read <drive> <lba> <sectors>");
        return;
    };
    let (Ok(drive), Ok(lba), Ok(sectors)) = (
        drive.parse::<u8>(),
        lba.parse::<u64>(),
        sectors.parse::<usize>(),
    ) else {
        println!("usage: ata read wants three numbers");
        return;
    };
    if drive > 1 || sectors == 0 {
        println!("ata read wants drive 0..1 and at least one sector");
        return;
    }
    match ata.read(drive, lba, sectors) {
        Ok(payload) => println!(
            "read ok drive={drive} lba={lba} sectors={sectors} bytes={} sum={}",
            payload.len(),
            checksum(&payload)
        ),
        Err(error) => println!("ata read failed: {error:?}"),
    }
}

fn checksum(payload: &[u8]) -> u64 {
    payload
        .iter()
        .fold(0u64, |sum, byte| sum.wrapping_add(u64::from(*byte)))
}
