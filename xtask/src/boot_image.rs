use std::fs;
use std::path::{Path, PathBuf};
use std::process::{self, Command};

use rxpart::mbr::ENTRY_COUNT;
use rxpart::mbr::Entry;
use rxpart::mbr::SECTOR_BYTES;
use x86_runner::chain;

pub use x86_runner::chain::PARTITION_START_LBA;
pub use x86_runner::chain::STAGE2_IMAGE_LBA;

use crate::check_isa::on_path;
use crate::run;
use crate::workspace_root;

pub const STUB_SOURCE: &str = "arch/x86-bootloader/scripts/harness_stub.s";
pub const STAGE1_SOURCE: &str = "arch/x86-bootloader/stage1/boot.s";
pub const STAGE2_TARGET_SPEC: &str = "arch/x86-bootloader/rosx-boot-i486.json";
pub const BOOT_SECTOR_BYTES: usize = SECTOR_BYTES;

pub const BOOT_IMAGE_BYTES: u64 = 16 * 1024 * 1024;
pub const FLOPPY_IMAGE_BYTES: u64 = 1440 * 1024;

const PROBE_LOCAL_NAME: &str = "probe.txt";
const PROBE_VOLUME_NAME: &str = "PROBE.TXT";
const PROBE_CONTENTS: &str = "rosx boot-image probe file\n";
const INITRD_LOCAL_NAME: &str = "initrd.bin";
pub const INITRD_VOLUME_NAME: &str = "INITRD.BIN";
const KERNEL_LOCAL_NAME: &str = "kernel.elf";
pub const KERNEL_VOLUME_NAME: &str = "KERNEL.ELF";
pub const FAKE_KERNEL_CRATE: &str = "fake-kernel-32";
pub const FAKE_KERNEL_64_CRATE: &str = "fake-kernel-64";
pub const FAKE_KERNEL_64_TARGET_SPEC: &str = "arch/x86-bootloader/rosx-boot-x86_64.json";

pub const INITRD_SEED: u64 = 0xB007_7E73_2026_1107;
pub const INITRD_BYTES: usize = 64 * 1024;

#[derive(Copy, Clone)]
pub enum Stage2Flavor {
    Flat32,
    Long64,
}

impl Stage2Flavor {
    fn feature(self) -> Option<&'static str> {
        match self {
            Stage2Flavor::Flat32 => None,
            Stage2Flavor::Long64 => Some("stage-64"),
        }
    }

    fn blob_name(self) -> &'static str {
        match self {
            Stage2Flavor::Flat32 => "stage2-32.bin",
            Stage2Flavor::Long64 => "stage2-64.bin",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Stage2Flavor::Flat32 => "stage2-32",
            Stage2Flavor::Long64 => "stage2-64",
        }
    }
}

pub fn initrd_payload() -> Vec<u8> {
    initrd_payload_of(INITRD_BYTES)
}

pub fn initrd_payload_of(bytes: usize) -> Vec<u8> {
    let mut payload = vec![0u8; bytes];
    let mut state = INITRD_SEED;
    for chunk in payload.chunks_mut(8) {
        state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut mixed = state;
        mixed = (mixed ^ (mixed >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        mixed = (mixed ^ (mixed >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        mixed ^= mixed >> 31;
        chunk.copy_from_slice(&mixed.to_le_bytes()[..chunk.len()]);
    }
    payload
}

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

    fn stage2_flavor(self) -> Stage2Flavor {
        match self {
            BootImage::Boot32 | BootImage::Floppy => Stage2Flavor::Flat32,
            BootImage::Boot64 => Stage2Flavor::Long64,
        }
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
    let missing: Vec<&str> = ["as", "ld", "objdump", "objcopy", "mformat", "mcopy", "mdir"]
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

fn link_flat(object: &Path, flat: &Path, text: &str, flavor: &str) -> Vec<u8> {
    let mut linker = Command::new("ld");
    linker.args(["-m", "elf_i386", "-Ttext", text, "-e", "_start", "--oformat", "binary", "-o"]);
    linker.arg(flat).arg(object);
    run(&mut linker, &format!("link the {flavor} into a flat binary"));
    fs::read(flat).expect("cannot read the linked flat binary")
}

fn link_flat_boot_sector(object: &Path, flat: &Path, flavor: &str) -> Vec<u8> {
    let sector = link_flat(object, flat, "0x7C00", flavor);
    assert_eq!(
        sector.len(),
        BOOT_SECTOR_BYTES,
        "the {flavor} must be exactly one boot sector"
    );
    assert_eq!(
        &sector[BOOT_SECTOR_BYTES - 2..],
        &[0x55, 0xAA],
        "the {flavor} must carry the boot signature"
    );
    sector
}

pub fn build_stage2(object_dir: &Path, flavor: Stage2Flavor) -> Vec<u8> {
    let mut builder = crate::cargo();
    builder
        .arg("build")
        .arg("--release")
        .arg("-p")
        .arg("rosx-stage2")
        .arg("--target")
        .arg(STAGE2_TARGET_SPEC)
        .args(crate::UNSTABLE);
    if let Some(feature) = flavor.feature() {
        builder.args(["--features", feature]);
    }
    run(&mut builder, &format!("build the {} image crate", flavor.label()));
    let elf = workspace_root()
        .join("target")
        .join("rosx-boot-i486")
        .join("release")
        .join("rosx-stage2");
    let flat = object_dir.join(flavor.blob_name());
    let mut objcopy = Command::new("objcopy");
    objcopy.args(["-O", "binary"]).arg(&elf).arg(&flat);
    run(&mut objcopy, &format!("objcopy the {} ELF into a flat binary", flavor.label()));
    let bin = fs::read(&flat).expect("cannot read the flat stage2 binary");
    assert!(
        !bin.is_empty() && bin.len() <= chain::STAGE2_MAX_BIN_BYTES,
        "the stage2 image is {} bytes; the 0x7E00..0x18000 budget is {}",
        bin.len(),
        chain::STAGE2_MAX_BIN_BYTES
    );
    match flavor {
        Stage2Flavor::Flat32 => {
            let offenders = crate::check_isa::audit_stage_binary(&flat).expect("stage2 ISA audit");
            assert!(
                offenders.is_empty(),
                "the stage2 image fails the 486 stage deny-list audit: {}",
                offenders.join(", ")
            );
        }
        Stage2Flavor::Long64 => {
            let elf_copy = object_dir.join("stage2-64.elf");
            fs::copy(&elf, &elf_copy).expect("cannot copy the stage2-64 ELF for the audit");
            let offenders = crate::check_isa::audit_stage2_longmode(&elf_copy)
                .expect("stage2-64 per-section ISA audit");
            assert!(
                offenders.is_empty(),
                "the stage2-64 image fails the per-section audit: {}",
                offenders.join(", ")
            );
        }
    }
    bin
}

pub fn build_stage1(object_dir: &Path, hard_unit: bool) -> Vec<u8> {
    let object = object_dir.join("stage1.o");
    let flat = object_dir.join("stage1.bin");
    let mut assembler = Command::new("as");
    assembler
        .arg("--32")
        .arg(format!("--defsym=HARD_UNIT={}", u8::from(hard_unit)))
        .arg(workspace_root().join(STAGE1_SOURCE))
        .arg("-o")
        .arg(&object);
    run(&mut assembler, "assemble the stage1 boot sector");
    let stage1 = link_flat_boot_sector(&object, &flat, "stage1");
    let offenders = crate::check_isa::audit_stage_binary(&flat).expect("stage1 ISA audit");
    assert!(
        offenders.is_empty(),
        "stage1 fails the 486 stage deny-list audit: {}",
        offenders.join(", ")
    );
    stage1
}

pub fn build_flat_stub(variant: StubVariant, object_dir: &Path) -> Vec<u8> {
    let name = variant.name();
    let object = object_dir.join(format!("harness-{name}.o"));
    let flat = object_dir.join(format!("harness-{name}.bin"));
    let mut assembler = Command::new("as");
    assembler
        .arg("--32")
        .arg(format!("--defsym=VARIANT={}", variant.defsym_value()))
        .arg(workspace_root().join(STUB_SOURCE))
        .arg("-o")
        .arg(&object);
    run(&mut assembler, &format!("assemble the {name} harness stub"));
    link_flat_boot_sector(&object, &flat, &format!("{name} harness stub"))
}

pub fn pack(kind: BootImage) -> PathBuf {
    let image = kind.path();
    match kind {
        BootImage::Boot32 | BootImage::Boot64 => pack_partitioned(image.clone(), kind),
        BootImage::Floppy => pack_floppy(image.clone()),
    }
    image
}

fn pack_partitioned(image: PathBuf, kind: BootImage) {
    let stage1 = build_stage1(&scratch_dir(), true);

    let mut entries: [Option<Entry>; ENTRY_COUNT] = [None; ENTRY_COUNT];
    entries[0] = Some(Entry {
        boot: chain::BOOTABLE,
        type_id: chain::FAT16_TYPE,
        start_lba: PARTITION_START_LBA,
        sector_count: (BOOT_IMAGE_BYTES / SECTOR_BYTES as u64) as u32 - PARTITION_START_LBA,
    });
    let mbr = chain::boot_mbr(&stage1, &entries);

    chain::create_sparse(&image, BOOT_IMAGE_BYTES);
    chain::write_at(&image, 0, &mbr);
    let stage2 = build_stage2(&scratch_dir(), kind.stage2_flavor());
    chain::write_at(
        &image,
        u64::from(STAGE2_IMAGE_LBA) * SECTOR_BYTES as u64,
        &chain::stage2_blob(&stage2),
    );

    let partition_sectors = (BOOT_IMAGE_BYTES / SECTOR_BYTES as u64) as u32 - PARTITION_START_LBA;
    chain::mformat_volume(&image, PARTITION_START_LBA, partition_sectors);
    mcopy_probe(&image);
    mcopy_initrd(&image);
    match kind {
        BootImage::Boot64 => mcopy_kernel(&image, build_fake_kernel64()),
        _ => mcopy_kernel(&image, build_fake_kernel()),
    }
    mdir_sees_the_boot_volume(&image);
}

fn pack_floppy(image: PathBuf) {
    let stage1 = build_stage1(&scratch_dir(), false);
    chain::create_sparse(&image, FLOPPY_IMAGE_BYTES);
    chain::write_at(&image, 0, &stage1);
    let stage2 = build_stage2(&scratch_dir(), Stage2Flavor::Flat32);
    chain::write_at(
        &image,
        u64::from(STAGE2_IMAGE_LBA) * SECTOR_BYTES as u64,
        &chain::stage2_blob(&stage2),
    );
}

fn mcopy_probe(image: &Path) {
    let probe = scratch_dir().join(PROBE_LOCAL_NAME);
    fs::write(&probe, PROBE_CONTENTS).expect("cannot write the volume probe file");
    chain::mcopy_file(image, PARTITION_START_LBA, &probe, &format!("::{PROBE_VOLUME_NAME}"));
}

fn mcopy_initrd(image: &Path) {
    let initrd = scratch_dir().join(INITRD_LOCAL_NAME);
    fs::write(&initrd, initrd_payload()).expect("cannot write the volume initrd file");
    chain::mcopy_file(image, PARTITION_START_LBA, &initrd, &format!("::{INITRD_VOLUME_NAME}"));
}

pub fn build_fake_kernel() -> Vec<u8> {
    let mut builder = crate::cargo();
    builder
        .arg("build")
        .arg("--release")
        .arg("-p")
        .arg(FAKE_KERNEL_CRATE)
        .arg("--target")
        .arg(STAGE2_TARGET_SPEC)
        .args(crate::UNSTABLE);
    run(&mut builder, "build the fake-kernel-32 smoke kernel");
    let elf = workspace_root()
        .join("target")
        .join("rosx-boot-i486")
        .join("release")
        .join(FAKE_KERNEL_CRATE);
    fs::read(&elf).expect("cannot read the fake-kernel-32 ELF")
}

pub fn build_fake_kernel64() -> Vec<u8> {
    let mut builder = crate::cargo();
    builder
        .arg("build")
        .arg("--release")
        .arg("-p")
        .arg(FAKE_KERNEL_64_CRATE)
        .arg("--target")
        .arg(FAKE_KERNEL_64_TARGET_SPEC)
        .args(crate::UNSTABLE);
    run(&mut builder, "build the fake-kernel-64 smoke kernel");
    let elf = workspace_root()
        .join("target")
        .join("rosx-boot-x86_64")
        .join("release")
        .join(FAKE_KERNEL_64_CRATE);
    fs::read(&elf).expect("cannot read the fake-kernel-64 ELF")
}

fn mcopy_kernel(image: &Path, kernel_elf: Vec<u8>) {
    let kernel = scratch_dir().join(KERNEL_LOCAL_NAME);
    fs::write(&kernel, kernel_elf).expect("cannot write the kernel ELF scratch file");
    chain::mcopy_file(image, PARTITION_START_LBA, &kernel, &format!("::{KERNEL_VOLUME_NAME}"));
}

fn mdir_sees_the_boot_volume(image: &Path) {
    let listing = chain::mdir_listing(image, PARTITION_START_LBA);
    assert!(
        listing.contains("PROBE")
            && listing.contains("TXT")
            && listing.contains("KERNEL")
            && listing.contains("ELF"),
        "mdir cannot see the boot volume files on the {label} volume at LBA {start} — the FAT16 \
         recipe is broken; output: {listing}",
        label = chain::VOLUME_LABEL,
        start = PARTITION_START_LBA,
    );
    println!(
        "==> mdir confirms {probe} and {kernel} on {label} at LBA {start}",
        probe = PROBE_VOLUME_NAME,
        kernel = KERNEL_VOLUME_NAME,
        label = chain::VOLUME_LABEL,
        start = PARTITION_START_LBA,
    );
}

pub(crate) fn scratch_dir() -> PathBuf {
    let dir = workspace_root().join("target").join("boot-test");
    fs::create_dir_all(&dir).expect("cannot create target/boot-test");
    dir
}

pub fn boot32_dir() -> PathBuf {
    let dir = workspace_root().join("target").join("boot32");
    fs::create_dir_all(&dir).expect("cannot create target/boot32");
    dir
}

pub fn stage_boot32_artifacts() -> (Vec<u8>, Vec<u8>) {
    require_boot_image_tools();
    let dir = boot32_dir();
    let stage1 = build_stage1(&dir, true);
    let stage2 = build_stage2(&dir, Stage2Flavor::Flat32);
    println!("==> staged stage1 and stage2-32 in {}", dir.display());
    (stage1, stage2)
}
