use std::fs::{self, File, OpenOptions};
use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::{self, Command};

use rxpart::mbr::{Entry, ENTRY_COUNT, SECTOR_BYTES, TABLE_OFFSET, encode_mbr};

use crate::check_isa::on_path;
use crate::run;
use crate::workspace_root;

pub const STUB_SOURCE: &str = "arch/x86-bootloader/scripts/harness_stub.s";
pub const BOOT_SECTOR_BYTES: usize = SECTOR_BYTES;

pub const BOOT_IMAGE_BYTES: u64 = 16 * 1024 * 1024;
pub const FLOPPY_IMAGE_BYTES: u64 = 1440 * 1024;

pub const PARTITION_START_LBA: u32 = 2048;
pub const STAGE2_IMAGE_LBA: u32 = 8;
pub const STAGE2_HEADER_MAGIC: &[u8; 4] = b"ROS2";

const BOOTABLE: u8 = 0x80;
const FAT16_TYPE: u8 = 0x06;
const VOLUME_LABEL: &str = "ROSXBOOT";
const HEADS: &str = "16";
const SECTORS_PER_TRACK: &str = "63";
const SECTORS_PER_CLUSTER: &str = "2";
const PROBE_LOCAL_NAME: &str = "probe.txt";
const PROBE_VOLUME_NAME: &str = "PROBE.TXT";
const PROBE_CONTENTS: &str = "rosx boot-image probe file\n";

#[derive(Copy, Clone)]
pub enum StubVariant {
    Pass,
    Fail,
    Crash,
    Hang,
}

impl StubVariant {
    pub fn name(self) -> &'static str {
        match self {
            StubVariant::Pass => "pass",
            StubVariant::Fail => "fail",
            StubVariant::Crash => "crash",
            StubVariant::Hang => "hang",
        }
    }

    fn defsym_value(self) -> u8 {
        match self {
            StubVariant::Pass => 0,
            StubVariant::Fail => 1,
            StubVariant::Crash => 2,
            StubVariant::Hang => 3,
        }
    }
}

#[derive(Copy, Clone)]
pub enum BootImage {
    Boot32,
    Boot64,
    Floppy,
}

impl BootImage {
    pub fn from_token(token: &str) -> Option<BootImage> {
        match token {
            "32" => Some(BootImage::Boot32),
            "64" => Some(BootImage::Boot64),
            "floppy" => Some(BootImage::Floppy),
            _ => None,
        }
    }

    pub fn path(self) -> PathBuf {
        let name = match self {
            BootImage::Boot32 => "boot32.img",
            BootImage::Boot64 => "boot64.img",
            BootImage::Floppy => "floppy.img",
        };
        workspace_root().join("target").join(name)
    }
}

pub fn boot_image_command(token: &str) {
    let Some(kind) = BootImage::from_token(token) else {
        eprintln!("boot-image expects 32, 64 or floppy, got: {token}");
        process::exit(2);
    };
    require_boot_image_tools();
    let image = pack(kind);
    println!("==> wrote {} boot image {}", token, image.display());
}

pub fn require_boot_image_tools() {
    let missing: Vec<&str> = ["as", "ld", "mformat", "mcopy", "mdir"]
        .into_iter()
        .filter(|tool| on_path(tool).is_none())
        .collect();
    if !missing.is_empty() {
        eprintln!("boot-image needs host tools that are missing:");
        for tool in missing {
            eprintln!("  {tool}");
        }
        process::exit(1);
    }
}

pub fn build_flat_stub(variant: StubVariant, object_dir: &Path) -> Vec<u8> {
    let object = object_dir.join(format!("harness-{}.o", variant.name()));
    let flat = object_dir.join(format!("harness-{}.bin", variant.name()));

    let mut assembler = Command::new("as");
    assembler
        .arg("--32")
        .arg(format!("--defsym=VARIANT={}", variant.defsym_value()))
        .arg(workspace_root().join(STUB_SOURCE))
        .arg("-o")
        .arg(&object);
    run(
        &mut assembler,
        &format!("assemble the {} harness stub", variant.name()),
    );

    let mut linker = Command::new("ld");
    linker
        .arg("-m")
        .arg("elf_i386")
        .arg("-Ttext")
        .arg("0x7C00")
        .arg("-e")
        .arg("_start")
        .arg("--oformat")
        .arg("binary")
        .arg("-o")
        .arg(&flat)
        .arg(&object);
    run(
        &mut linker,
        &format!("link the {} harness stub into a flat boot sector", variant.name()),
    );

    let stub = fs::read(&flat).expect("cannot read the linked harness stub");
    assert_eq!(
        stub.len(),
        BOOT_SECTOR_BYTES,
        "the {} harness stub must be exactly one boot sector",
        variant.name()
    );
    assert_eq!(
        &stub[BOOT_SECTOR_BYTES - 2..],
        &[0x55, 0xAA],
        "the {} harness stub must carry the boot signature",
        variant.name(),
    );
    stub
}

pub fn pack(kind: BootImage) -> PathBuf {
    let image = kind.path();
    match kind {
        BootImage::Boot32 | BootImage::Boot64 => pack_partitioned(image.clone()),
        BootImage::Floppy => pack_floppy(image.clone()),
    }
    image
}

fn pack_partitioned(image: PathBuf) {
    let stub = build_flat_stub(StubVariant::Pass, &scratch_dir());
    assert!(
        stub[TABLE_OFFSET..BOOT_SECTOR_BYTES - 2]
            .iter()
            .all(|&byte| byte == 0),
        "the placeholder stage1 reaches the partition table zone of its sector"
    );

    let mut entries: [Option<Entry>; ENTRY_COUNT] = [None; ENTRY_COUNT];
    entries[0] = Some(Entry {
        boot: BOOTABLE,
        type_id: FAT16_TYPE,
        start_lba: PARTITION_START_LBA,
        sector_count: (BOOT_IMAGE_BYTES / SECTOR_BYTES as u64) as u32 - PARTITION_START_LBA,
    });
    let mut mbr = encode_mbr(&entries).to_vec();
    mbr[..TABLE_OFFSET].copy_from_slice(&stub[..TABLE_OFFSET]);

    create_sparse(&image, BOOT_IMAGE_BYTES);
    write_at(&image, 0, &mbr);
    write_at(&image, u64::from(STAGE2_IMAGE_LBA) * SECTOR_BYTES as u64, &stage2_placeholder());

    let partition_sectors = (BOOT_IMAGE_BYTES / SECTOR_BYTES as u64) as u32 - PARTITION_START_LBA;
    mformat_volume(&image, partition_sectors);
    mcopy_probe(&image);
    mdir_sees_probe(&image);
}

fn pack_floppy(image: PathBuf) {
    let stub = build_flat_stub(StubVariant::Pass, &scratch_dir());
    create_sparse(&image, FLOPPY_IMAGE_BYTES);
    write_at(&image, 0, &stub);
    write_at(&image, u64::from(STAGE2_IMAGE_LBA) * SECTOR_BYTES as u64, &stage2_placeholder());
}

fn stage2_placeholder() -> Vec<u8> {
    let total_bytes = (2 * SECTOR_BYTES) as u32;
    let mut blob = vec![0u8; 2 * SECTOR_BYTES];
    blob[0..4].copy_from_slice(STAGE2_HEADER_MAGIC);
    blob[4..8].copy_from_slice(&0u32.to_le_bytes());
    blob[8..12].copy_from_slice(&total_bytes.to_le_bytes());
    blob
}

fn partition_index(image: &Path) -> String {
    format!(
        "-i{}@@{}",
        image.display(),
        u64::from(PARTITION_START_LBA) * SECTOR_BYTES as u64
    )
}

fn mformat_volume(image: &Path, partition_sectors: u32) {
    let mut cmd = Command::new("mformat");
    cmd.arg(partition_index(image));
    cmd.args(["-h", HEADS, "-s", SECTORS_PER_TRACK]);
    cmd.args(["-T", &partition_sectors.to_string()]);
    cmd.args(["-c", SECTORS_PER_CLUSTER, "-v", VOLUME_LABEL]);
    run(&mut cmd, "mformat the boot image FAT16 volume at LBA 2048");
}

fn mcopy_probe(image: &Path) {
    let probe = scratch_dir().join(PROBE_LOCAL_NAME);
    fs::write(&probe, PROBE_CONTENTS).expect("cannot write the volume probe file");
    let mut cmd = Command::new("mcopy");
    cmd.arg("-m")
        .arg("-o")
        .arg(partition_index(image))
        .arg(&probe)
        .arg(format!("::{PROBE_VOLUME_NAME}"));
    run(&mut cmd, "mcopy the probe file onto the boot image volume");
}

fn mdir_sees_probe(image: &Path) {
    let mut cmd = Command::new("mdir");
    cmd.arg(partition_index(image)).arg("::");
    let output = cmd.output().expect("failed to run mdir");
    let listing = String::from_utf8_lossy(&output.stdout).to_string();
    assert!(
        output.status.success()
            && listing.contains("ROSXBOOT")
            && listing.contains("PROBE")
            && listing.contains("TXT"),
        "mdir cannot see the probe file on the {label} volume at LBA {start} — the FAT16 \
         recipe is broken; output: {listing}",
        label = VOLUME_LABEL,
        start = PARTITION_START_LBA,
    );
    println!(
        "==> mdir confirms {probe} on {label} at LBA {start}",
        probe = PROBE_VOLUME_NAME,
        label = VOLUME_LABEL,
        start = PARTITION_START_LBA,
    );
}

fn scratch_dir() -> PathBuf {
    let dir = workspace_root().join("target").join("boot-test");
    fs::create_dir_all(&dir).expect("cannot create target/boot-test");
    dir
}

fn create_sparse(path: &Path, bytes: u64) {
    let file = File::create(path).expect("failed to create the boot image");
    file.set_len(bytes)
        .expect("failed to size the boot image sparse file");
}

fn write_at(path: &Path, offset: u64, bytes: &[u8]) {
    let mut file = OpenOptions::new()
        .write(true)
        .open(path)
        .expect("failed to open the boot image for writing");
    file.seek(SeekFrom::Start(offset))
        .expect("failed to seek the boot image");
    file.write_all(bytes)
        .expect("failed to write a region of the boot image");
}
