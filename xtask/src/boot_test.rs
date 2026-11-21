use std::fs::{self, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::{self, Command, ExitStatus, Stdio};
use std::thread::sleep;
use std::time::{Duration, Instant};

use crate::boot_image::{
    self, BootImage, StubVariant, build_flat_stub, require_boot_image_tools,
};
use crate::check_isa::on_path;
use crate::workspace_root;

const HARNESS_IMAGE_BYTES: u64 = 2 * 1024 * 1024;

struct BootMachine {
    binary: &'static str,
    cpu: &'static str,
    ram: &'static str,
    ram_bytes: u64,
}

const MEBIBYTE: u64 = 1024 * 1024;

const BIOS_32: BootMachine = BootMachine {
    binary: "qemu-system-i386",
    cpu: "486",
    ram: "32M",
    ram_bytes: 32 * MEBIBYTE,
};

const BIOS_32_512: BootMachine = BootMachine {
    binary: "qemu-system-i386",
    cpu: "486",
    ram: "512M",
    ram_bytes: 512 * MEBIBYTE,
};

const BIOS_32_128: BootMachine = BootMachine {
    binary: "qemu-system-i386",
    cpu: "486",
    ram: "128M",
    ram_bytes: 128 * MEBIBYTE,
};

const BIOS_32_8: BootMachine = BootMachine {
    binary: "qemu-system-i386",
    cpu: "486",
    ram: "8M",
    ram_bytes: 8 * MEBIBYTE,
};

const BIOS_64: BootMachine = BootMachine {
    binary: "qemu-system-x86_64",
    cpu: "qemu64",
    ram: "128M",
    ram_bytes: 128 * MEBIBYTE,
};

const BIOS_64_512: BootMachine = BootMachine {
    binary: "qemu-system-x86_64",
    cpu: "qemu64",
    ram: "512M",
    ram_bytes: 512 * MEBIBYTE,
};

const BIOS_64_NO_LM: BootMachine = BootMachine {
    binary: "qemu-system-x86_64",
    cpu: "qemu64,-lm",
    ram: "128M",
    ram_bytes: 128 * MEBIBYTE,
};

#[derive(Copy, Clone)]
enum BootMedia {
    Disk,
    Floppy,
}

#[derive(Copy, Clone)]
enum ImageSource {
    HarnessStub(StubVariant),
    BootImage(BootImage),
    CorruptStage2Header,
    ScribbledFatRoot,
    PatchedKernel(KernelPatch),
    Elf32OnLongMode,
    OversizedInitrd,
    NoInitrdVolume,
    BuiltBootDisk,
}

#[derive(Clone, Copy)]
enum KernelPatch {
    Elf64Class,
    ZeroPhysicalAddress,
}

#[derive(Copy, Clone, Debug, PartialEq)]
enum ExitVerdict {
    Success,
    Failure,
    CrashedBeforeVerdict,
    TimedOut,
    Unexpected(i32),
}

impl ExitVerdict {
    fn describe(self) -> String {
        match self {
            ExitVerdict::Success => "exit 33 (success)".to_string(),
            ExitVerdict::Failure => "exit 3 (guest-reported failure)".to_string(),
            ExitVerdict::CrashedBeforeVerdict => "crashed before verdict".to_string(),
            ExitVerdict::TimedOut => "timed out".to_string(),
            ExitVerdict::Unexpected(code) => format!("unexpected exit code {code}"),
        }
    }
}

struct BootCheckpoint {
    name: &'static str,
    machine: &'static BootMachine,
    media: BootMedia,
    source: ImageSource,
    expected: ExitVerdict,
    console_contains: &'static [&'static str],
    console_absent: &'static [&'static str],
    timeout_seconds: u64,
}

const CHECKPOINTS: [BootCheckpoint; 25] = [
    BootCheckpoint {
        name: "harness-pass",
        machine: &BIOS_32,
        media: BootMedia::Disk,
        source: ImageSource::HarnessStub(StubVariant::Pass),
        expected: ExitVerdict::Success,
        console_contains: &["BOOTLOADER-HARNESS OK"],
        console_absent: &["BOOTLOADER-HARNESS FAIL"],
        timeout_seconds: 5,
    },
    BootCheckpoint {
        name: "harness-fail",
        machine: &BIOS_32,
        media: BootMedia::Disk,
        source: ImageSource::HarnessStub(StubVariant::Fail),
        expected: ExitVerdict::Failure,
        console_contains: &["BOOTLOADER-HARNESS FAIL"],
        console_absent: &["BOOTLOADER-HARNESS OK"],
        timeout_seconds: 5,
    },
    BootCheckpoint {
        name: "harness-crash",
        machine: &BIOS_32,
        media: BootMedia::Disk,
        source: ImageSource::HarnessStub(StubVariant::Crash),
        expected: ExitVerdict::CrashedBeforeVerdict,
        console_contains: &[],
        console_absent: &["BOOTLOADER-HARNESS OK", "BOOTLOADER-HARNESS FAIL"],
        timeout_seconds: 5,
    },
    BootCheckpoint {
        name: "harness-hang",
        machine: &BIOS_32,
        media: BootMedia::Disk,
        source: ImageSource::HarnessStub(StubVariant::Hang),
        expected: ExitVerdict::TimedOut,
        console_contains: &[],
        console_absent: &["BOOTLOADER-HARNESS OK", "BOOTLOADER-HARNESS FAIL"],
        timeout_seconds: 2,
    },
    BootCheckpoint {
        name: "stage2-pm-486",
        machine: &BIOS_32,
        media: BootMedia::Disk,
        source: ImageSource::BootImage(BootImage::Boot32),
        expected: ExitVerdict::Success,
        console_contains: &[
            "[STAGE1] OK",
            "[STAGE2_32] A20=OK THUNK=OK",
            "[MEM88]",
            "[INT13]",
            "[DISK] media=lba",
        ],
        console_absent: &["BOOTLOADER-HARNESS", "[ERR] disk"],
        timeout_seconds: 2,
    },
    BootCheckpoint {
        name: "stage2-hdd-lba-x86_64",
        machine: &BIOS_64,
        media: BootMedia::Disk,
        source: ImageSource::BootImage(BootImage::Boot64),
        expected: ExitVerdict::Success,
        console_contains: &[
            "[STAGE1] OK",
            "[STAGE2_64] A20=OK THUNK=OK",
            "[MEM88]",
            "[INT13]",
            "[DISK] media=lba",
        ],
        console_absent: &["BOOTLOADER-HARNESS", "[ERR] disk"],
        timeout_seconds: 2,
    },
    BootCheckpoint {
        name: "stage2-floppy-chs",
        machine: &BIOS_32,
        media: BootMedia::Floppy,
        source: ImageSource::BootImage(BootImage::Floppy),
        expected: ExitVerdict::Success,
        console_contains: &[
            "[STAGE1] OK",
            "[STAGE2_32] A20=OK THUNK=OK",
            "[MEM88]",
            "[INT13]",
        ],
        console_absent: &["BOOTLOADER-HARNESS", "[ERR] disk", "[DISK]"],
        timeout_seconds: 2,
    },
    BootCheckpoint {
        name: "stage2-map-512m",
        machine: &BIOS_32_512,
        media: BootMedia::Disk,
        source: ImageSource::BootImage(BootImage::Boot32),
        expected: ExitVerdict::Success,
        console_contains: &[
            "[STAGE1] OK",
            "[STAGE2_32] A20=OK THUNK=OK",
            "[MEM] entries=",
            "[DISK] media=lba",
        ],
        console_absent: &["BOOTLOADER-HARNESS", "[ERR] disk", "[ERR] memnorm"],
        timeout_seconds: 3,
    },
    BootCheckpoint {
        name: "handoff32",
        machine: &BIOS_32,
        media: BootMedia::Disk,
        source: ImageSource::BootImage(BootImage::Boot32),
        expected: ExitVerdict::Success,
        console_contains: &[
            "[INITRD] addr=",
            "kernel loaded",
            "arch: x86_32",
            "initrd_crc32: 0x",
        ],
        console_absent: &["[FAT-ERR]", "[ERR] mem", "[FK32] violation"],
        timeout_seconds: 3,
    },
    BootCheckpoint {
        name: "handoff32-512m",
        machine: &BIOS_32_512,
        media: BootMedia::Disk,
        source: ImageSource::BootImage(BootImage::Boot32),
        expected: ExitVerdict::Success,
        console_contains: &["[INITRD] addr=0x", "kernel loaded", "initrd_crc32: 0x"],
        console_absent: &["[FAT-ERR]", "[ERR] mem", "[FK32] violation"],
        timeout_seconds: 4,
    },
    BootCheckpoint {
        name: "handoff32-no-initrd",
        machine: &BIOS_32,
        media: BootMedia::Disk,
        source: ImageSource::NoInitrdVolume,
        expected: ExitVerdict::Success,
        console_contains: &[
            "[INITRD] addr=0x00000000 size=0",
            "kernel loaded",
            "arch: x86_32",
            "initrd_crc32: 0x00000000",
        ],
        console_absent: &["[FAT-ERR]", "[ERR] mem", "[FK32] violation"],
        timeout_seconds: 3,
    },
    BootCheckpoint {
        name: "handoff64",
        machine: &BIOS_64,
        media: BootMedia::Disk,
        source: ImageSource::BootImage(BootImage::Boot64),
        expected: ExitVerdict::Success,
        console_contains: &[
            "[INITRD] addr=",
            "kernel loaded",
            "arch: x86_64",
            "initrd_crc32: 0x",
        ],
        console_absent: &["[FAT-ERR]", "[ERR] mem", "[FK64] violation"],
        timeout_seconds: 3,
    },
    BootCheckpoint {
        name: "handoff64-512m",
        machine: &BIOS_64_512,
        media: BootMedia::Disk,
        source: ImageSource::BootImage(BootImage::Boot64),
        expected: ExitVerdict::Success,
        console_contains: &["[INITRD] addr=", "kernel loaded", "initrd_crc32: 0x"],
        console_absent: &["[FAT-ERR]", "[ERR] mem", "[FK64] violation"],
        timeout_seconds: 4,
    },
    BootCheckpoint {
        name: "initrd-too-big",
        machine: &BIOS_32_8,
        media: BootMedia::Disk,
        source: ImageSource::OversizedInitrd,
        expected: ExitVerdict::Failure,
        console_contains: &["[ELF]", "[ERR] mem"],
        console_absent: &["[ERR] memnorm", "[INITRD]", "kernel loaded", "[STAGE2_32]"],
        timeout_seconds: 3,
    },
    BootCheckpoint {
        name: "fat-boot64-media",
        machine: &BIOS_64,
        media: BootMedia::Disk,
        source: ImageSource::BootImage(BootImage::Boot64),
        expected: ExitVerdict::Success,
        console_contains: &["[INITRD] addr="],
        console_absent: &["[FAT-ERR]"],
        timeout_seconds: 3,
    },
    BootCheckpoint {
        name: "fat-corrupt",
        machine: &BIOS_32,
        media: BootMedia::Disk,
        source: ImageSource::ScribbledFatRoot,
        expected: ExitVerdict::Failure,
        console_contains: &["[FAT-ERR]"],
        console_absent: &["[INITRD]", "[STAGE2_32]"],
        timeout_seconds: 3,
    },
    BootCheckpoint {
        name: "elf32-load",
        machine: &BIOS_32,
        media: BootMedia::Disk,
        source: ImageSource::BootImage(BootImage::Boot32),
        expected: ExitVerdict::Success,
        console_contains: &[
            "[ELF] arch=X86_32 entry=0x00100000 phys=0x00100000..",
            "kernel loaded",
        ],
        console_absent: &["[ERR] elf"],
        timeout_seconds: 3,
    },
    BootCheckpoint {
        name: "elf32-class64",
        machine: &BIOS_32,
        media: BootMedia::Disk,
        source: ImageSource::PatchedKernel(KernelPatch::Elf64Class),
        expected: ExitVerdict::Failure,
        console_contains: &["[ERR] elf class"],
        console_absent: &["[ELF]", "kernel loaded"],
        timeout_seconds: 3,
    },
    BootCheckpoint {
        name: "elf32-zero-paddr",
        machine: &BIOS_32,
        media: BootMedia::Disk,
        source: ImageSource::PatchedKernel(KernelPatch::ZeroPhysicalAddress),
        expected: ExitVerdict::Failure,
        console_contains: &["[ERR] elf layout"],
        console_absent: &["[INITRD]", "kernel loaded"],
        timeout_seconds: 3,
    },
    BootCheckpoint {
        name: "elf64-class32",
        machine: &BIOS_64,
        media: BootMedia::Disk,
        source: ImageSource::Elf32OnLongMode,
        expected: ExitVerdict::Failure,
        console_contains: &["[DISK] media=lba", "[ERR] elf class"],
        console_absent: &["[ELF] arch=", "kernel loaded"],
        timeout_seconds: 3,
    },
    BootCheckpoint {
        name: "lm-smoke",
        machine: &BIOS_64,
        media: BootMedia::Disk,
        source: ImageSource::BootImage(BootImage::Boot64),
        expected: ExitVerdict::Success,
        console_contains: &[
            "[STAGE1] OK",
            "[STAGE2_64] A20=OK THUNK=OK",
            "[ELF] arch=X86_64 entry=0xffff800000100000",
            "[INITRD] addr=",
            "[PAGETABLE] root=",
            "kernel loaded",
        ],
        console_absent: &["[FAT-ERR]", "[ERR]", "[FK64] violation"],
        timeout_seconds: 3,
    },
    BootCheckpoint {
        name: "lm-guard-486",
        machine: &BIOS_32,
        media: BootMedia::Disk,
        source: ImageSource::BootImage(BootImage::Boot64),
        expected: ExitVerdict::Failure,
        console_contains: &["[STAGE1] OK", "[ERR] CPU does not support x86_64 Long Mode"],
        console_absent: &["[STAGE2_64]", "[FK64]", "[MEM88]"],
        timeout_seconds: 3,
    },
    BootCheckpoint {
        name: "lm-guard-nolm",
        machine: &BIOS_64_NO_LM,
        media: BootMedia::Disk,
        source: ImageSource::BootImage(BootImage::Boot64),
        expected: ExitVerdict::Failure,
        console_contains: &["[STAGE1] OK", "[ERR] CPU does not support x86_64 Long Mode"],
        console_absent: &["[STAGE2_64]", "[FK64]", "[MEM88]"],
        timeout_seconds: 3,
    },
    BootCheckpoint {
        name: "stage2-corrupt-header",
        machine: &BIOS_32,
        media: BootMedia::Disk,
        source: ImageSource::CorruptStage2Header,
        expected: ExitVerdict::TimedOut,
        console_contains: &["[STAGE1] OK", "[ERR] disk"],
        console_absent: &["[STAGE2_32] A20=OK", "BOOTLOADER-HARNESS"],
        timeout_seconds: 2,
    },
    BootCheckpoint {
        name: "real-kernel-boot32",
        machine: &BIOS_32_128,
        media: BootMedia::Disk,
        source: ImageSource::BuiltBootDisk,
        expected: ExitVerdict::TimedOut,
        console_contains: &[
            "[STAGE1] OK",
            "[STAGE2_32] A20=OK THUNK=OK",
            "[BOOTINFO] regions=",
            "[KERNEL] Bootstrapped",
            "[KERNEL] Initializing",
            "[KERNEL] Starting",
        ],
        console_absent: &["!!! PANIC !!!", "bad Multiboot magic", "unknown boot magic"],
        timeout_seconds: 5,
    },
];

struct BootOutcome {
    verdict: ExitVerdict,
    console: String,
}

pub fn run_boot_test() {
    require_host_tools();
    let mut failures = 0;
    let mut skipped = 0;
    for checkpoint in &CHECKPOINTS {
        let image = match prepare_image(&checkpoint.source) {
            Prepared::Image(image) => image,
            Prepared::Skipped(reason) => {
                skipped += 1;
                println!("SKIP {} — {reason}", checkpoint.name);
                continue;
            }
        };
        let outcome = boot(checkpoint.machine, checkpoint.media, &image, checkpoint.timeout_seconds);
        match verdict_mismatch(checkpoint, &outcome) {
            None => println!("PASS {} — {}", checkpoint.name, outcome.verdict.describe()),
            Some(reason) => {
                failures += 1;
                println!("FAIL {} — {reason}", checkpoint.name);
                println!("  outcome: {}", outcome.verdict.describe());
                dump_console(&outcome.console);
            }
        }
    }
    println!(
        "==> boot-test: {} checkpoints, {} failures, {} skipped",
        CHECKPOINTS.len(),
        failures,
        skipped
    );
    if failures > 0 {
        process::exit(1);
    }
}

fn require_host_tools() {
    require_boot_image_tools();
    let missing: Vec<&str> = ["qemu-system-i386", "qemu-system-x86_64", "mdel"]
        .into_iter()
        .filter(|tool| on_path(tool).is_none())
        .collect();
    if !missing.is_empty() {
        eprintln!("boot-test needs host tools that are missing:");
        for tool in missing {
            eprintln!("  {tool}");
        }
        process::exit(1);
    }
}

enum Prepared {
    Image(PathBuf),
    Skipped(&'static str),
}

fn prepare_image(source: &ImageSource) -> Prepared {
    match source {
        ImageSource::HarnessStub(variant) => Prepared::Image(pack_harness_stub_image(*variant)),
        ImageSource::BootImage(kind) => Prepared::Image(boot_image::pack(*kind)),
        ImageSource::CorruptStage2Header => Prepared::Image(pack_corrupt_header_image()),
        ImageSource::ScribbledFatRoot => Prepared::Image(pack_scribbled_fat_root_image()),
        ImageSource::PatchedKernel(patch) => Prepared::Image(pack_patched_kernel_image(*patch)),
        ImageSource::Elf32OnLongMode => Prepared::Image(pack_elf32_on_long_mode_image()),
        ImageSource::OversizedInitrd => Prepared::Image(pack_oversized_initrd_image()),
        ImageSource::NoInitrdVolume => Prepared::Image(pack_no_initrd_image()),
        ImageSource::BuiltBootDisk => match built_boot_disk() {
            Some(image) => Prepared::Image(image),
            None => Prepared::Skipped(
                "no target/rosx-i486-boot-disk.img for the real kernel; run `cargo xtask build x86_32` first",
            ),
        },
    }
}

fn built_boot_disk() -> Option<PathBuf> {
    let spec = crate::arch::find("x86_32").expect("x86_32 is a registered arch");
    let disk = crate::boot_disk::boot_disk_path(spec);
    disk.is_file().then_some(disk)
}

fn pack_corrupt_header_image() -> PathBuf {
    let image = workspace_root()
        .join("target")
        .join("boot-test")
        .join("stage2-corrupt-header.img");
    fs::copy(boot_image::pack(BootImage::Boot32), &image)
        .expect("cannot copy the boot32 image for header corruption");
    let mut file = OpenOptions::new()
        .write(true)
        .open(&image)
        .expect("cannot open the corrupt-header image");
    file.seek(SeekFrom::Start(
        u64::from(boot_image::STAGE2_IMAGE_LBA) * boot_image::BOOT_SECTOR_BYTES as u64,
    ))
    .expect("cannot seek to the stage2 header");
    file.write_all(b"NOPE")
        .expect("cannot scribble the stage2 header magic");
    image
}

fn pack_scribbled_fat_root_image() -> PathBuf {
    let image = workspace_root()
        .join("target")
        .join("boot-test")
        .join("stage2-scribbled-fat-root.img");
    fs::copy(boot_image::pack(BootImage::Boot32), &image)
        .expect("cannot copy the boot32 image for root-directory scribbling");
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&image)
        .expect("cannot open the scribbled-fat-root image");
    let sector = boot_image::BOOT_SECTOR_BYTES as u64;
    file.seek(SeekFrom::Start(u64::from(boot_image::PARTITION_START_LBA) * sector))
        .expect("cannot seek to the volume boot sector");
    let mut boot_sector = [0u8; 512];
    file.read_exact(&mut boot_sector)
        .expect("cannot read the volume boot sector");
    let reserved_sectors = u16::from_le_bytes([boot_sector[14], boot_sector[15]]) as u64;
    let fat_sectors = u16::from_le_bytes([boot_sector[22], boot_sector[23]]) as u64;
    let root_entries = u16::from_le_bytes([boot_sector[17], boot_sector[18]]) as u64;
    let root_lba = u64::from(boot_image::PARTITION_START_LBA) + reserved_sectors + 2 * fat_sectors;
    file.seek(SeekFrom::Start(root_lba * sector))
        .expect("cannot seek to the FAT root directory");
    file.write_all(&vec![0u8; (root_entries * 32) as usize])
        .expect("cannot scribble the FAT root directory");
    image
}

fn pack_patched_kernel_image(patch: KernelPatch) -> PathBuf {
    let image = workspace_root()
        .join("target")
        .join("boot-test")
        .join(match patch {
            KernelPatch::Elf64Class => "stage2-elf64-class.img",
            KernelPatch::ZeroPhysicalAddress => "stage2-elf-zero-paddr.img",
        });
    fs::copy(boot_image::pack(BootImage::Boot32), &image)
        .expect("cannot copy the boot32 image for kernel patching");
    let mut elf = boot_image::build_fake_kernel();
    match patch {
        KernelPatch::Elf64Class => patch_identity_to_elf64(&mut elf),
        KernelPatch::ZeroPhysicalAddress => zero_first_load_physical_address(&mut elf),
    }
    mcopy_kernel_onto(&image, &elf);
    image
}

fn patch_identity_to_elf64(elf: &mut [u8]) {
    elf[4] = 2;
    elf[18..20].copy_from_slice(&0x003Eu16.to_le_bytes());
}

fn zero_first_load_physical_address(elf: &mut [u8]) {
    let (table, entry_size, count) = kernel_program_headers(elf);
    for index in 0..count {
        let record = table + index * entry_size;
        let kind = u32::from_le_bytes(elf[record..record + 4].try_into().unwrap());
        if kind == PT_LOAD {
            elf[record + 12..record + 16].copy_from_slice(&0u32.to_le_bytes());
            return;
        }
    }
    panic!("the fake kernel ELF carries no PT_LOAD to corrupt");
}

fn mcopy_kernel_onto(image: &Path, elf: &[u8]) {
    mcopy_volume_file(image, "patched-kernel.elf", boot_image::KERNEL_VOLUME_NAME, elf);
}

fn pack_elf32_on_long_mode_image() -> PathBuf {
    let image = workspace_root()
        .join("target")
        .join("boot-test")
        .join("stage2-elf32-on-64.img");
    fs::copy(boot_image::pack(BootImage::Boot64), &image)
        .expect("cannot copy the boot64 image for the class-1 kernel swap");
    let elf = boot_image::build_fake_kernel();
    mcopy_kernel_onto(&image, &elf);
    image
}

fn mcopy_volume_file(image: &Path, local_name: &str, volume_name: &str, bytes: &[u8]) {
    let scratch = workspace_root()
        .join("target")
        .join("boot-test")
        .join(local_name);
    fs::write(&scratch, bytes).expect("cannot write the mcopy scratch file");
    let offset = u64::from(boot_image::PARTITION_START_LBA) * boot_image::BOOT_SECTOR_BYTES as u64;
    let mut cmd = Command::new("mcopy");
    cmd.arg("-m")
        .arg("-o")
        .arg(format!("-i{}@@{}", image.display(), offset))
        .arg(&scratch)
        .arg(format!("::{volume_name}"));
    let status = cmd.status().expect("failed to run mcopy");
    assert!(
        status.success(),
        "mcopy cannot replace {volume_name} on the test image"
    );
}

fn pack_oversized_initrd_image() -> PathBuf {
    let image = workspace_root()
        .join("target")
        .join("boot-test")
        .join("stage2-oversized-initrd.img");
    fs::copy(boot_image::pack(BootImage::Boot32), &image)
        .expect("cannot copy the boot32 image for the oversized initrd");
    mcopy_volume_file(
        &image,
        "oversized-initrd.bin",
        boot_image::INITRD_VOLUME_NAME,
        &boot_image::initrd_payload_of(8 * MEBIBYTE as usize),
    );
    image
}

fn pack_no_initrd_image() -> PathBuf {
    let image = workspace_root()
        .join("target")
        .join("boot-test")
        .join("stage2-no-initrd.img");
    fs::copy(boot_image::pack(BootImage::Boot32), &image)
        .expect("cannot copy the boot32 image for the absent initrd");
    mdelete_volume_file(&image, boot_image::INITRD_VOLUME_NAME);
    image
}

fn mdelete_volume_file(image: &Path, volume_name: &str) {
    let offset = u64::from(boot_image::PARTITION_START_LBA) * boot_image::BOOT_SECTOR_BYTES as u64;
    let mut cmd = Command::new("mdel");
    cmd.arg("-i")
        .arg(format!("{}@@{}", image.display(), offset))
        .arg(format!("::{volume_name}"));
    let status = cmd.status().expect("failed to run mdel");
    assert!(
        status.success(),
        "mdel cannot remove {volume_name} from the test image"
    );
}

fn pack_harness_stub_image(variant: StubVariant) -> PathBuf {
    let dir = workspace_root().join("target").join("boot-test");
    fs::create_dir_all(&dir).expect("cannot create target/boot-test");
    let mut blob = build_flat_stub(variant, &dir);
    blob.resize(HARNESS_IMAGE_BYTES as usize, 0);
    let image = dir.join(format!("harness-{}.img", variant.name()));
    fs::write(&image, &blob).expect("cannot write the harness image");
    image
}

fn boot(
    machine: &BootMachine,
    media: BootMedia,
    image: &Path,
    timeout_seconds: u64,
) -> BootOutcome {
    let console_path = image.with_extension("console");
    let _ = fs::remove_file(&console_path);
    let drive = match media {
        BootMedia::Disk => format!("format=raw,file={}", image.display()),
        BootMedia::Floppy => format!("format=raw,if=floppy,file={}", image.display()),
    };
    let debugcon = format!("file:{}", console_path.display());
    let mut child = Command::new(machine.binary)
        .args([
            "-cpu",
            machine.cpu,
            "-m",
            machine.ram,
            "-drive",
            &drive,
            "-debugcon",
            &debugcon,
            "-display",
            "none",
            "-monitor",
            "none",
            "-device",
            "isa-debug-exit,iobase=0xf4,iosize=0x04",
            "-no-reboot",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("failed to spawn qemu");

    let deadline = Instant::now() + Duration::from_secs(timeout_seconds);
    let verdict = loop {
        if let Some(status) = child.try_wait().expect("failed to poll qemu") {
            break classify_status(status);
        }
        if Instant::now() >= deadline {
            child.kill().expect("failed to kill qemu");
            child.wait().expect("failed to reap qemu");
            break ExitVerdict::TimedOut;
        }
        sleep(Duration::from_millis(25));
    };
    BootOutcome {
        verdict,
        console: fs::read_to_string(&console_path).unwrap_or_default(),
    }
}

fn classify_status(status: ExitStatus) -> ExitVerdict {
    match status.code() {
        Some(33) => ExitVerdict::Success,
        Some(3) => ExitVerdict::Failure,
        Some(0) | Some(1) => ExitVerdict::CrashedBeforeVerdict,
        Some(other) => ExitVerdict::Unexpected(other),
        None => ExitVerdict::CrashedBeforeVerdict,
    }
}

fn verdict_mismatch(checkpoint: &BootCheckpoint, outcome: &BootOutcome) -> Option<String> {
    if outcome.verdict != checkpoint.expected {
        return Some(format!(
            "expected {}, got {}",
            checkpoint.expected.describe(),
            outcome.verdict.describe()
        ));
    }
    for expected_text in checkpoint.console_contains {
        if !outcome.console.contains(expected_text) {
            return Some(format!("console output is missing {expected_text:?}"));
        }
    }
    for forbidden_text in checkpoint.console_absent {
        if outcome.console.contains(forbidden_text) {
            return Some(format!("console output unexpectedly contains {forbidden_text:?}"));
        }
    }
    if outcome.console.contains("[MEM] entries=")
        && let Some(reason) = verify_memory_map(&outcome.console, checkpoint.machine.ram_bytes)
    {
        return Some(reason);
    }
    if outcome.console.contains("kernel loaded")
        && let Some(line) = outcome
            .console
            .lines()
            .find(|line| line.starts_with("[INITRD]"))
        && let Some(reason) = verify_initrd(line, &outcome.console)
    {
        return Some(reason);
    }
    None
}

fn verify_initrd(line: &str, console: &str) -> Option<String> {
    let mut fields = line.split_whitespace();
    let (_, _) = (fields.next()?, fields.next()?);
    let (Some(addr), Some(size), Some(printed)) = (
        parse_hex_field(fields.next()?, "addr="),
        parse_dec_field(fields.next()?, "size="),
        parse_hex_field(fields.next()?, "crc32="),
    ) else {
        return Some(format!("malformed [INITRD] line: {line}"));
    };
    let host = if size == 0 {
        if addr != 0 {
            return Some(format!(
                "zero-size initrd carries address {addr:#010x}, the contract demands 0"
            ));
        }
        0u64
    } else {
        let expected_address = if console.contains("arch: x86_64") {
            fake_kernel64_phys_end()
        } else {
            fake_kernel_phys_end()
        };
        if addr != expected_address {
            return Some(format!(
                "initrd placed at {addr:#010x}, expected the kernel phys end {expected_address:#010x}"
            ));
        }
        if size != boot_image::INITRD_BYTES as u64 {
            return Some(format!(
                "initrd read {size} bytes, expected {}",
                boot_image::INITRD_BYTES
            ));
        }
        u64::from(crc32_ieee(&boot_image::initrd_payload()))
    };
    if printed != host {
        return Some(format!(
            "stage2 crc32 {printed:#010x} differs from the host-seeded {host:#010x}"
        ));
    }
    let Some(kernel_printed) = console
        .lines()
        .find_map(|candidate| candidate.strip_prefix("initrd_crc32: "))
    else {
        return Some(
            "the fake kernel never reported its independently recomputed initrd crc32".to_string(),
        );
    };
    let kernel_crc = u64::from_str_radix(kernel_printed.trim_start_matches("0x"), 16).ok()?;
    if kernel_crc != printed {
        return Some(format!(
            "kernel-recomputed crc32 {kernel_crc:#010x} differs from the stage2 {printed:#010x}"
        ));
    }
    None
}

const PT_LOAD: u32 = 1;

fn kernel_program_headers(elf: &[u8]) -> (usize, usize, usize) {
    let table = u32::from_le_bytes(elf[28..32].try_into().unwrap()) as usize;
    let entry_size = u16::from_le_bytes(elf[42..44].try_into().unwrap()) as usize;
    let count = u16::from_le_bytes(elf[44..46].try_into().unwrap()) as usize;
    (table, entry_size, count)
}

fn kernel64_program_headers(elf: &[u8]) -> (usize, usize, usize) {
    let table = u64::from_le_bytes(elf[32..40].try_into().unwrap()) as usize;
    let entry_size = u16::from_le_bytes(elf[54..56].try_into().unwrap()) as usize;
    let count = u16::from_le_bytes(elf[56..58].try_into().unwrap()) as usize;
    (table, entry_size, count)
}

fn fake_kernel_phys_end() -> u64 {
    let elf = boot_image::build_fake_kernel();
    let (table, entry_size, count) = kernel_program_headers(&elf);
    let mut phys_end = 0u64;
    for index in 0..count {
        let record = table + index * entry_size;
        if u32::from_le_bytes(elf[record..record + 4].try_into().unwrap()) != PT_LOAD {
            continue;
        }
        let address = u32::from_le_bytes(elf[record + 12..record + 16].try_into().unwrap()) as u64;
        let memory_bytes =
            u32::from_le_bytes(elf[record + 20..record + 24].try_into().unwrap()) as u64;
        phys_end = phys_end.max(address + memory_bytes);
    }
    (phys_end + 4095) & !4095
}

fn fake_kernel64_phys_end() -> u64 {
    let elf = boot_image::build_fake_kernel64();
    let (table, entry_size, count) = kernel64_program_headers(&elf);
    let mut phys_end = 0u64;
    for index in 0..count {
        let record = table + index * entry_size;
        if u32::from_le_bytes(elf[record..record + 4].try_into().unwrap()) != PT_LOAD {
            continue;
        }
        let address = u64::from_le_bytes(elf[record + 24..record + 32].try_into().unwrap());
        let memory_bytes = u64::from_le_bytes(elf[record + 40..record + 48].try_into().unwrap());
        phys_end = phys_end.max(address + memory_bytes);
    }
    (phys_end + 4095) & !4095
}

fn parse_hex_field(field: &str, key: &str) -> Option<u64> {
    u64::from_str_radix(field.strip_prefix(key)?.strip_prefix("0x")?, 16).ok()
}

fn parse_dec_field(field: &str, key: &str) -> Option<u64> {
    field.strip_prefix(key)?.parse().ok()
}

fn crc32_ieee(data: &[u8]) -> u32 {
    let mut accumulator = 0xFFFF_FFFFu32;
    for byte in data {
        accumulator ^= u32::from(*byte);
        for _ in 0..8 {
            let mask = (accumulator & 1).wrapping_neg();
            accumulator = (accumulator >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !accumulator
}

fn dump_console(console: &str) {
    for line in console.lines() {
        println!("  console | {line}");
    }
}

const FIRST_MEBIBYTE: u64 = MEBIBYTE;

#[derive(Clone, Copy)]
struct MapRow {
    base: u64,
    length: u64,
    kind: u32,
}

struct MapSummary {
    entries: u64,
    raw_sum: u64,
    norm_sum: u64,
    usable: u64,
    gap: u64,
}

struct Reservation {
    start: u64,
    end: u64,
    kind: u32,
}

const RESERVATION_TABLE: [Reservation; 11] = [
    Reservation { start: 0x0000_0000, end: 0x0000_0400, kind: 2 },
    Reservation { start: 0x0000_0400, end: 0x0000_0500, kind: 2 },
    Reservation { start: 0x0000_0500, end: 0x0000_0600, kind: 3 },
    Reservation { start: 0x0000_0600, end: 0x0000_0800, kind: 3 },
    Reservation { start: 0x0000_0800, end: 0x0000_7C00, kind: 3 },
    Reservation { start: 0x0000_7C00, end: 0x0000_7E00, kind: 3 },
    Reservation { start: 0x0000_7E00, end: 0x0003_0000, kind: 3 },
    Reservation { start: 0x0003_0000, end: 0x0003_1000, kind: 2 },
    Reservation { start: 0x0003_1000, end: 0x0007_C000, kind: 1 },
    Reservation { start: 0x0007_C000, end: 0x0008_0000, kind: 2 },
    Reservation { start: 0x0008_0000, end: FIRST_MEBIBYTE, kind: 2 },
];

fn verify_memory_map(console: &str, ram_bytes: u64) -> Option<String> {
    let rows = parse_map_rows(console)?;
    let summary = parse_map_summary(console)?;
    if rows.is_empty() {
        return Some("memory map printed no [MEM] region rows".to_string());
    }
    if summary.entries as usize != rows.len() {
        return Some(format!(
            "summary claims {} entries, console printed {}",
            summary.entries,
            rows.len()
        ));
    }
    if summary.raw_sum != summary.norm_sum {
        return Some(format!(
            "normalization changed the byte total: raw_sum={:#x} norm_sum={:#x}",
            summary.raw_sum, summary.norm_sum
        ));
    }
    if rows.iter().any(|row| row.length == 0) {
        return Some("memory map carries a zero-length region".to_string());
    }
    for pair in rows.windows(2) {
        let (previous, next) = (pair[0], pair[1]);
        if next.base < previous.base + previous.length {
            return Some(format!(
                "regions overlap or are unsorted: {:#x}..{:#x} then {:#x}..{:#x}",
                previous.base,
                previous.base + previous.length,
                next.base,
                next.base + next.length
            ));
        }
    }
    let recomputed_norm_sum: u64 = rows.iter().map(|row| row.length).sum();
    if recomputed_norm_sum != summary.norm_sum {
        return Some(format!(
            "row lengths sum to {:#x}, summary says norm_sum={:#x}",
            recomputed_norm_sum, summary.norm_sum
        ));
    }
    let recomputed_usable: u64 = rows
        .iter()
        .filter(|row| row.kind == 1)
        .map(|row| row.length)
        .sum();
    if recomputed_usable != summary.usable {
        return Some(format!(
            "usable rows sum to {:#x}, summary says usable={:#x}",
            recomputed_usable, summary.usable
        ));
    }
    if rows
        .iter()
        .any(|row| !matches!(row.kind, 1 | 2 | 3 | 5) && row.base < FIRST_MEBIBYTE)
    {
        return Some("memory map carries a kind outside the boot ABI".to_string());
    }
    for row in rows.iter().filter(|row| row.base < FIRST_MEBIBYTE) {
        let Some(expected) = reservation_kind(row.base) else {
            return Some(format!(
                "map row {:#x}..{:#x} sits outside the reservation table",
                row.base,
                row.base + row.length
            ));
        };
        if row.kind != expected {
            return Some(format!(
                "reservation table demands kind {} at {:#x}, map says kind {}",
                expected, row.base, row.kind
            ));
        }
    }
    let usable_above_one_mebibyte = rows
        .iter()
        .filter(|row| row.kind == 1 && row.base >= FIRST_MEBIBYTE)
        .map(|row| row.length)
        .sum::<u64>();
    let top_of_usable = rows
        .iter()
        .filter(|row| row.kind == 1)
        .map(|row| row.base + row.length)
        .max()
        .unwrap_or(0);
    let expected_gap = top_of_usable
        .saturating_sub(FIRST_MEBIBYTE)
        .saturating_sub(usable_above_one_mebibyte);
    if expected_gap != summary.gap {
        return Some(format!(
            "recomputed gap {:#x} differs from printed gap={:#x}",
            expected_gap, summary.gap
        ));
    }
    if summary.gap > 4 * MEBIBYTE {
        return Some(format!(
            "bios gap {:#x} exceeds the 4 MiB allowance",
            summary.gap
        ));
    }
    if !(top_of_usable <= ram_bytes && ram_bytes - top_of_usable <= MEBIBYTE) {
        return Some(format!(
            "top of usable {:#x} is off the machine's {:#x} bytes of RAM",
            top_of_usable, ram_bytes
        ));
    }
    if usable_above_one_mebibyte == 0 {
        return Some("memory map reports no usable memory above 1 MiB".to_string());
    }
    None
}

fn reservation_kind(address: u64) -> Option<u32> {
    RESERVATION_TABLE
        .iter()
        .find(|slot| slot.start <= address && address < slot.end)
        .map(|slot| slot.kind)
}

fn parse_hex_value(token: &str) -> Option<u64> {
    let digits = token.strip_prefix("0x")?;
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    u64::from_str_radix(digits, 16).ok()
}

fn parse_map_rows(console: &str) -> Option<Vec<MapRow>> {
    let mut rows = Vec::new();
    for line in console.lines().map(|line| line.trim_end_matches('\r')) {
        let Some(rest) = line.strip_prefix("[MEM] base=") else {
            continue;
        };
        let mut fields = rest.split_whitespace();
        let base = parse_hex_value(fields.next()?)?;
        if fields.next()? != "len=" {
            return None;
        }
        let length = parse_hex_value(fields.next()?)?;
        if fields.next()? != "kind=" {
            return None;
        }
        let kind = fields.next()?.parse::<u32>().ok()?;
        if fields.next().is_some() {
            return None;
        }
        rows.push(MapRow { base, length, kind });
    }
    Some(rows)
}

fn parse_map_summary(console: &str) -> Option<MapSummary> {
    let line = console
        .lines()
        .map(|line| line.trim_end_matches('\r'))
        .find(|line| line.starts_with("[MEM] entries="))?;
    let mut entries = None;
    let mut raw_sum = None;
    let mut norm_sum = None;
    let mut usable = None;
    let mut gap = None;
    let mut tokens = line.strip_prefix("[MEM] ")?.split_whitespace();
    while let Some(token) = tokens.next() {
        match token {
            "entries=" => entries = Some(tokens.next()?.parse::<u64>().ok()?),
            "raw_sum=" => raw_sum = parse_hex_value(tokens.next()?),
            "norm_sum=" => norm_sum = parse_hex_value(tokens.next()?),
            "usable=" => usable = parse_hex_value(tokens.next()?),
            "gap=" => gap = parse_hex_value(tokens.next()?),
            "OK" => {}
            _ => return None,
        }
    }
    Some(MapSummary {
        entries: entries?,
        raw_sum: raw_sum?,
        norm_sum: norm_sum?,
        usable: usable?,
        gap: gap?,
    })
}
