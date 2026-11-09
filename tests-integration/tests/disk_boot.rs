use std::path::{Path, PathBuf};
use std::time::Duration;
use tests_integration::arch::Arch;
use tests_integration::fat_disk::{built_elf, tools_available, FatDisk};
use tests_integration::qemu::QemuSession;

const BOOT_BUDGET: Duration = Duration::from_secs(120);
const OP_BUDGET: Duration = Duration::from_secs(60);
const DISK_BYTES: usize = 12 * 1024 * 1024;
const GREETING: &[u8] = b"greetings from the FAT root\n";
const MENU: &[u8] = b"boot files live under /boot\n";

fn skip_reason() -> Option<&'static str> {
    if Arch::current() != Arch::X86_32 {
        return Some("the FAT boot disk flow belongs to the x86_32 port");
    }
    if !tools_available() {
        return Some("mtools not installed, skipping the FAT boot disk flow");
    }
    None
}

fn run(session: &mut QemuSession, command: &str) -> String {
    let mark = session.mark();
    session.send_line(command);
    session.expect_output_since(mark, "rose>", OP_BUDGET);
    session.content_since(mark)
}

fn build_boot_disk(staging: &Path, with_apps: bool) -> FatDisk {
    let disk = FatDisk::create(staging, "boot-disk.img", DISK_BYTES);
    disk.write_seed(GREETING, "::greeting.txt");
    disk.make_directory("::/boot");
    disk.write_seed(MENU, "::/boot/menu.txt");
    if with_apps {
        for app in ["shell", "mkdir", "randomd"] {
            let elf = built_elf(app);
            disk.copy(&elf, &format!("::{app}"));
        }
    }
    disk
}

#[test]
fn boot_disk_mounts_the_root_and_hands_off_the_disk_shell() {
    if let Some(reason) = skip_reason() {
        eprintln!("skipped: {reason}");
        return;
    }
    let staging = tempfile::tempdir().expect("tempdir for the boot disk image");
    let disk = build_boot_disk(staging.path(), true);

    let mut session = QemuSession::spawn_with_master_disk(disk.path());
    session.expect_output("attach registered ATA", BOOT_BUDGET);
    session.expect_output("mount ok device=ata0d0p0 dir=/", BOOT_BUDGET);
    session.expect_output("[INIT] starting /randomd", BOOT_BUDGET);
    session.expect_output("[INIT] RANDOM ready", BOOT_BUDGET);
    session.expect_output("[INIT] /echo not available", BOOT_BUDGET);
    session.expect_output("[INIT] handing over to /shell", BOOT_BUDGET);
    session.expect_output("rose>", BOOT_BUDGET);

    let listing = run(&mut session, "ls");
    assert!(
        listing.contains("greeting.txt"),
        "the FAT root must serve the filesystem: {listing}"
    );
    assert!(
        !listing.contains("vfsd"),
        "the root mount must shadow the ROM roster: {listing}"
    );

    let boot_listing = run(&mut session, "ls /boot");
    assert!(
        boot_listing.contains("menu.txt"),
        "nested FAT directories must read through the root mount: {boot_listing}"
    );

    run(&mut session, "mkdir /data");
    let listing = run(&mut session, "ls");
    assert!(
        listing.contains("data"),
        "the FAT root mount must be writable: {listing}"
    );
}

fn built_boot_disk() -> Option<PathBuf> {
    let image = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("target")
        .join("rosx-i486-boot-disk.img");
    image.is_file().then_some(image)
}

#[test]
fn the_built_boot_disk_boots_to_the_disk_shell() {
    if Arch::current() != Arch::X86_32 || !tools_available() {
        eprintln!("skipped: the built boot disk belongs to the x86_32 flow with mtools");
        return;
    }
    let Some(image) = built_boot_disk() else {
        eprintln!("skipped: no target/rosx-i486-boot-disk.img, run `cargo xtask build x86_32` first");
        return;
    };
    let mut session = QemuSession::spawn_with_master_disk(&image);
    session.expect_output("mount ok device=ata0d0p0 dir=/", BOOT_BUDGET);
    session.expect_output("[INIT] starting /randomd", BOOT_BUDGET);
    session.expect_output("[INIT] RANDOM ready", BOOT_BUDGET);
    session.expect_output("[INIT] starting /pong", BOOT_BUDGET);
    session.expect_output("[INIT] PING ready", BOOT_BUDGET);
    session.expect_output("[INIT] handing over to /shell", BOOT_BUDGET);
    session.expect_output("rose>", BOOT_BUDGET);

    let listing = run(&mut session, "ls");
    assert!(
        listing.contains("snake") && listing.contains("tetris"),
        "the built disk carries every app through the root mount: {listing}"
    );
}

#[test]
fn a_mounted_root_without_a_shell_falls_back_to_the_rom_shell() {
    if let Some(reason) = skip_reason() {
        eprintln!("skipped: {reason}");
        return;
    }
    let staging = tempfile::tempdir().expect("tempdir for the boot disk image");
    let disk = build_boot_disk(staging.path(), false);

    let mut session = QemuSession::spawn_with_master_disk(disk.path());
    session.expect_output("mount ok device=ata0d0p0 dir=/", BOOT_BUDGET);
    session.expect_output("[INIT] hand-off /shell unavailable", BOOT_BUDGET);
    session.expect_output("[INIT] handing over to rom:shell", BOOT_BUDGET);
    session.expect_output("rose>", BOOT_BUDGET);

    let listing = run(&mut session, "ls");
    assert!(
        listing.contains("greeting.txt"),
        "the fallback shell still sees the mounted FAT root: {listing}"
    );
}
