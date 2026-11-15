use std::fs::{self, OpenOptions};
use std::io::{Seek, SeekFrom, Write};
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
}

const BIOS_32: BootMachine = BootMachine {
    binary: "qemu-system-i386",
    cpu: "486",
    ram: "32M",
};

const BIOS_64: BootMachine = BootMachine {
    binary: "qemu-system-x86_64",
    cpu: "qemu64",
    ram: "128M",
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
    serial_contains: &'static [&'static str],
    serial_absent: &'static [&'static str],
    timeout_seconds: u64,
}

const CHECKPOINTS: [BootCheckpoint; 8] = [
    BootCheckpoint {
        name: "harness-pass",
        machine: &BIOS_32,
        media: BootMedia::Disk,
        source: ImageSource::HarnessStub(StubVariant::Pass),
        expected: ExitVerdict::Success,
        serial_contains: &["BOOTLOADER-HARNESS OK"],
        serial_absent: &["BOOTLOADER-HARNESS FAIL"],
        timeout_seconds: 5,
    },
    BootCheckpoint {
        name: "harness-fail",
        machine: &BIOS_32,
        media: BootMedia::Disk,
        source: ImageSource::HarnessStub(StubVariant::Fail),
        expected: ExitVerdict::Failure,
        serial_contains: &["BOOTLOADER-HARNESS FAIL"],
        serial_absent: &["BOOTLOADER-HARNESS OK"],
        timeout_seconds: 5,
    },
    BootCheckpoint {
        name: "harness-crash",
        machine: &BIOS_32,
        media: BootMedia::Disk,
        source: ImageSource::HarnessStub(StubVariant::Crash),
        expected: ExitVerdict::CrashedBeforeVerdict,
        serial_contains: &[],
        serial_absent: &["BOOTLOADER-HARNESS OK", "BOOTLOADER-HARNESS FAIL"],
        timeout_seconds: 5,
    },
    BootCheckpoint {
        name: "harness-hang",
        machine: &BIOS_32,
        media: BootMedia::Disk,
        source: ImageSource::HarnessStub(StubVariant::Hang),
        expected: ExitVerdict::TimedOut,
        serial_contains: &[],
        serial_absent: &["BOOTLOADER-HARNESS OK", "BOOTLOADER-HARNESS FAIL"],
        timeout_seconds: 2,
    },
    BootCheckpoint {
        name: "stage2-pm-486",
        machine: &BIOS_32,
        media: BootMedia::Disk,
        source: ImageSource::BootImage(BootImage::Boot32),
        expected: ExitVerdict::Success,
        serial_contains: &["[STAGE1] OK", "[STAGE2_32] A20=OK"],
        serial_absent: &["BOOTLOADER-HARNESS", "[ERR] disk"],
        timeout_seconds: 2,
    },
    BootCheckpoint {
        name: "stage2-hdd-lba-x86_64",
        machine: &BIOS_64,
        media: BootMedia::Disk,
        source: ImageSource::BootImage(BootImage::Boot64),
        expected: ExitVerdict::Success,
        serial_contains: &["[STAGE1] OK", "[STAGE2_32] A20=OK"],
        serial_absent: &["BOOTLOADER-HARNESS", "[ERR] disk"],
        timeout_seconds: 2,
    },
    BootCheckpoint {
        name: "stage2-floppy-chs",
        machine: &BIOS_32,
        media: BootMedia::Floppy,
        source: ImageSource::BootImage(BootImage::Floppy),
        expected: ExitVerdict::Success,
        serial_contains: &["[STAGE1] OK", "[STAGE2_32] A20=OK"],
        serial_absent: &["BOOTLOADER-HARNESS", "[ERR] disk"],
        timeout_seconds: 2,
    },
    BootCheckpoint {
        name: "stage2-corrupt-header",
        machine: &BIOS_32,
        media: BootMedia::Disk,
        source: ImageSource::CorruptStage2Header,
        expected: ExitVerdict::TimedOut,
        serial_contains: &["[STAGE1] OK", "[ERR] disk"],
        serial_absent: &["[STAGE2_32] A20=OK", "BOOTLOADER-HARNESS"],
        timeout_seconds: 2,
    },
];

struct BootOutcome {
    verdict: ExitVerdict,
    serial: String,
}

pub fn run_boot_test() {
    require_host_tools();
    let mut failures = 0;
    for checkpoint in &CHECKPOINTS {
        let image = prepare_image(&checkpoint.source);
        let outcome = boot(checkpoint.machine, checkpoint.media, &image, checkpoint.timeout_seconds);
        match verdict_mismatch(checkpoint, &outcome) {
            None => println!("PASS {} — {}", checkpoint.name, outcome.verdict.describe()),
            Some(reason) => {
                failures += 1;
                println!("FAIL {} — {reason}", checkpoint.name);
                println!("  outcome: {}", outcome.verdict.describe());
                dump_serial(&outcome.serial);
            }
        }
    }
    println!(
        "==> boot-test: {} checkpoints, {} failures",
        CHECKPOINTS.len(),
        failures
    );
    if failures > 0 {
        process::exit(1);
    }
}

fn require_host_tools() {
    require_boot_image_tools();
    let missing: Vec<&str> = ["qemu-system-i386", "qemu-system-x86_64"]
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

fn prepare_image(source: &ImageSource) -> PathBuf {
    match source {
        ImageSource::HarnessStub(variant) => pack_harness_stub_image(*variant),
        ImageSource::BootImage(kind) => boot_image::pack(*kind),
        ImageSource::CorruptStage2Header => pack_corrupt_header_image(),
    }
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
    let serial_path = image.with_extension("serial");
    let _ = fs::remove_file(&serial_path);
    let drive = match media {
        BootMedia::Disk => format!("format=raw,file={}", image.display()),
        BootMedia::Floppy => format!("format=raw,if=floppy,file={}", image.display()),
    };
    let serial = format!("file:{}", serial_path.display());
    let mut child = Command::new(machine.binary)
        .args([
            "-cpu",
            machine.cpu,
            "-m",
            machine.ram,
            "-drive",
            &drive,
            "-serial",
            &serial,
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
        serial: fs::read_to_string(&serial_path).unwrap_or_default(),
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
    for expected_text in checkpoint.serial_contains {
        if !outcome.serial.contains(expected_text) {
            return Some(format!("serial output is missing {expected_text:?}"));
        }
    }
    for forbidden_text in checkpoint.serial_absent {
        if outcome.serial.contains(forbidden_text) {
            return Some(format!("serial output unexpectedly contains {forbidden_text:?}"));
        }
    }
    None
}

fn dump_serial(serial: &str) {
    for line in serial.lines() {
        println!("  serial | {line}");
    }
}
