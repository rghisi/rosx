use std::fs;
use std::path::{Path, PathBuf};
use std::process::{self, Command, ExitStatus, Stdio};
use std::thread::sleep;
use std::time::{Duration, Instant};

use crate::check_isa::on_path;
use crate::run;
use crate::workspace_root;

const STUB_SOURCE: &str = "arch/x86-bootloader/scripts/harness_stub.s";
const BOOT_SECTOR_BYTES: usize = 512;
const HARNESS_IMAGE_BYTES: usize = 2 * 1024 * 1024;

#[derive(Copy, Clone)]
enum StubVariant {
    Pass,
    Fail,
    Crash,
    Hang,
}

impl StubVariant {
    fn name(self) -> &'static str {
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
    variant: StubVariant,
    expected: ExitVerdict,
    serial_contains: &'static [&'static str],
    serial_absent: &'static [&'static str],
    timeout_seconds: u64,
}

const CHECKPOINTS: [BootCheckpoint; 4] = [
    BootCheckpoint {
        name: "harness-pass",
        variant: StubVariant::Pass,
        expected: ExitVerdict::Success,
        serial_contains: &["BOOTLOADER-HARNESS OK"],
        serial_absent: &["BOOTLOADER-HARNESS FAIL"],
        timeout_seconds: 5,
    },
    BootCheckpoint {
        name: "harness-fail",
        variant: StubVariant::Fail,
        expected: ExitVerdict::Failure,
        serial_contains: &["BOOTLOADER-HARNESS FAIL"],
        serial_absent: &["BOOTLOADER-HARNESS OK"],
        timeout_seconds: 5,
    },
    BootCheckpoint {
        name: "harness-crash",
        variant: StubVariant::Crash,
        expected: ExitVerdict::CrashedBeforeVerdict,
        serial_contains: &[],
        serial_absent: &["BOOTLOADER-HARNESS OK", "BOOTLOADER-HARNESS FAIL"],
        timeout_seconds: 5,
    },
    BootCheckpoint {
        name: "harness-hang",
        variant: StubVariant::Hang,
        expected: ExitVerdict::TimedOut,
        serial_contains: &[],
        serial_absent: &["BOOTLOADER-HARNESS OK", "BOOTLOADER-HARNESS FAIL"],
        timeout_seconds: 2,
    },
];

struct BootOutcome {
    verdict: ExitVerdict,
    serial: String,
}

pub fn run_boot_test() {
    require_host_tools();
    let run_dir = workspace_root().join("target").join("boot-test");
    fs::create_dir_all(&run_dir).expect("cannot create target/boot-test");

    let mut failures = 0;
    for checkpoint in &CHECKPOINTS {
        let image = pack_stub_image(checkpoint.variant, &run_dir);
        let outcome = boot(&image, checkpoint.timeout_seconds);
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
    let missing: Vec<&str> = ["as", "ld", "qemu-system-i386"]
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

fn pack_stub_image(variant: StubVariant, run_dir: &Path) -> PathBuf {
    let object = run_dir.join(format!("harness-{}.o", variant.name()));
    let flat = run_dir.join(format!("harness-{}.bin", variant.name()));

    let mut assembler = Command::new("as");
    assembler
        .arg("--32")
        .arg(format!("--defsym=VARIANT={}", variant.defsym_value()))
        .arg(workspace_root().join(STUB_SOURCE))
        .arg("-o")
        .arg(&object);
    run(&mut assembler, &format!("assemble the {} harness stub", variant.name()));

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

    let mut sector = fs::read(&flat).expect("cannot read the linked harness stub");
    assert_eq!(
        sector.len(),
        BOOT_SECTOR_BYTES,
        "the {} harness stub must be exactly one boot sector",
        variant.name()
    );
    assert_eq!(
        &sector[BOOT_SECTOR_BYTES - 2..],
        &[0x55, 0xAA],
        "the {} harness stub must carry the boot signature",
        variant.name()
    );

    let image = run_dir.join(format!("harness-{}.img", variant.name()));
    sector.resize(HARNESS_IMAGE_BYTES, 0);
    fs::write(&image, &sector).expect("cannot write the harness image");
    image
}

fn boot(image: &Path, timeout_seconds: u64) -> BootOutcome {
    let serial_path = image.with_extension("serial");
    let _ = fs::remove_file(&serial_path);
    let drive = format!("format=raw,file={}", image.display());
    let serial = format!("file:{}", serial_path.display());
    let mut child = Command::new("qemu-system-i386")
        .args([
            "-cpu",
            "486",
            "-m",
            "32M",
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
        .expect("failed to spawn qemu-system-i386");

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
