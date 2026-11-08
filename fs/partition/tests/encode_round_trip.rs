use rxpart::{encode_mbr, exposure, parse_mbr, Entry, Exposure, TableState, SECTOR_BYTES};
use std::fs::{self, File};
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

const DISK_SECTORS: u64 = 204_800;
const DISK_BYTES: u64 = DISK_SECTORS * SECTOR_BYTES as u64;

fn entry(boot: u8, type_id: u8, start_lba: u32, sector_count: u32) -> Entry {
    Entry {
        boot,
        type_id,
        start_lba,
        sector_count,
    }
}

fn sfdisk_present() -> bool {
    Command::new("sfdisk")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

fn write_image(shape: &str, entries: &[Option<Entry>; 4]) -> PathBuf {
    let path = std::env::temp_dir().join(format!("rxpart-compat-{shape}.img"));
    let mut file = File::create(&path).expect("cannot create round-trip image");
    file.write_all(&encode_mbr(entries)).unwrap();
    file.set_len(DISK_BYTES).unwrap();
    path
}

#[derive(Debug)]
struct ReadBack {
    slot: u8,
    start: u64,
    size: u64,
    type_id: u8,
    bootable: bool,
}

fn sfdisk_dump(image: &std::path::Path) -> String {
    let output = Command::new("sfdisk")
        .arg("--dump")
        .arg(image)
        .output()
        .expect("sfdisk --dump failed to launch");
    assert!(output.status.success(), "sfdisk --dump refused the image");
    String::from_utf8(output.stdout).unwrap()
}

fn read_back(dump: &str) -> Vec<ReadBack> {
    dump.lines()
        .filter_map(|line| {
            let (device, fields) = line.split_once(" : ")?;
            let slot = device.chars().next_back()?.to_digit(10)? as u8 - 1;
            let mut start = 0u64;
            let mut size = 0u64;
            let mut type_hex = None;
            let mut bootable = false;
            for field in fields.split(", ") {
                if let Some(value) = field.strip_prefix("start=") {
                    start = value.trim().parse().ok()?;
                } else if let Some(value) = field.strip_prefix("size=") {
                    size = value.trim().parse().ok()?;
                } else if let Some(value) = field.strip_prefix("type=") {
                    type_hex = Some(value);
                } else if field == "bootable" {
                    bootable = true;
                }
            }
            Some(ReadBack {
                slot,
                start,
                size,
                type_id: u8::from_str_radix(type_hex?, 16).ok()?,
                bootable,
            })
        })
        .collect()
}

fn sfdisk_verifies(image: &std::path::Path) {
    let status = Command::new("sfdisk")
        .arg("--verify")
        .arg(image)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .expect("sfdisk --verify failed to launch");
    assert!(status.success(), "sfdisk --verify rejected the table");
}

fn fdisk_reads_it(image: &std::path::Path) {
    let output = Command::new("fdisk")
        .arg("-l")
        .arg(image)
        .output()
        .expect("fdisk failed to launch");
    assert!(output.status.success(), "fdisk -l refused the image");
    let report = String::from_utf8(output.stdout).unwrap();
    assert!(report.contains("Disklabel type: dos"), "{report}");
}

fn round_trip(shape: &str, entries: &[Option<Entry>; 4], expected: &[(u8, u64, u64, u8, bool)]) {
    if !sfdisk_present() {
        eprintln!("sfdisk not installed: live read-back skipped");
        return;
    }
    let image = write_image(shape, entries);
    let table = parse_mbr(
        encode_mbr(entries).as_slice().try_into().unwrap(),
        DISK_SECTORS,
    );
    assert_eq!(table.state, TableState::Valid);
    assert_eq!(&table.entries, entries);
    let dumped = read_back(&sfdisk_dump(&image));
    let observed: Vec<(u8, u64, u64, u8, bool)> = dumped
        .iter()
        .map(|partition| (partition.slot, partition.start, partition.size, partition.type_id, partition.bootable))
        .collect();
    assert_eq!(observed, expected.to_vec());
    sfdisk_verifies(&image);
    fdisk_reads_it(&image);
    fs::remove_file(&image).unwrap();
}

#[test]
fn sfdisk_reads_back_the_single_bootable_partition_table() {
    round_trip(
        "one",
        &[
            Some(entry(0x80, 0x83, 2048, 100_352)),
            None,
            None,
            None,
        ],
        &[(0, 2048, 100_352, 0x83, true)],
    );
}

#[test]
fn sfdisk_reads_back_a_holed_table_with_true_slot_numbers() {
    round_trip(
        "hole",
        &[
            Some(entry(0x80, 0x0c, 2048, 50_000)),
            None,
            Some(entry(0x00, 0x82, 104_448, 50_000)),
            Some(entry(0x00, 0x83, 155_648, 40_000)),
        ],
        &[
            (0, 2048, 50_000, 0x0c, true),
            (2, 104_448, 50_000, 0x82, false),
            (3, 155_648, 40_000, 0x83, false),
        ],
    );
}

#[test]
fn sfdisk_reads_back_a_full_four_slot_table() {
    round_trip(
        "four",
        &[
            Some(entry(0x80, 0x0b, 2048, 50_000)),
            Some(entry(0x00, 0x83, 53_248, 50_000)),
            Some(entry(0x00, 0x82, 104_448, 50_000)),
            Some(entry(0x00, 0x16, 155_648, 30_000)),
        ],
        &[
            (0, 2048, 50_000, 0x0b, true),
            (1, 53_248, 50_000, 0x83, false),
            (2, 104_448, 50_000, 0x82, false),
            (3, 155_648, 30_000, 0x16, false),
        ],
    );
}

#[test]
fn sfdisk_reads_back_a_table_with_a_container_entry() {
    if !sfdisk_present() {
        eprintln!("sfdisk not installed: live read-back skipped");
        return;
    }
    let entries = [
        Some(entry(0x80, 0x83, 2048, 90_000)),
        Some(entry(0x00, 0x0f, 92_160, 100_000)),
        None,
        None,
    ];
    let image = write_image("container", &entries);
    let dumped = read_back(&sfdisk_dump(&image));
    let observed: Vec<(u8, u64, u64, u8)> = dumped
        .iter()
        .map(|partition| (partition.slot, partition.start, partition.size, partition.type_id))
        .collect();
    assert_eq!(
        observed,
        vec![
            (0, 2048, 90_000, 0x83),
            (1, 92_160, 100_000, 0x0f),
        ]
    );
    sfdisk_verifies(&image);
    let sector: [u8; SECTOR_BYTES] = encode_mbr(&entries);
    match exposure(&parse_mbr(&sector, DISK_SECTORS)) {
        Exposure::Partitions(slices) => assert_eq!(slices.iter().map(|slice| slice.slot).collect::<Vec<_>>(), vec![0]),
        Exposure::Whole { table } => panic!("container disk exposed whole with {table:?}"),
    }
    fs::remove_file(&image).unwrap();
}
