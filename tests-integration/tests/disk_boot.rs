use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;
use tests_integration::arch::Arch;
use tests_integration::qemu::QemuSession;

const BOOT_BUDGET: Duration = Duration::from_secs(120);
const OP_BUDGET: Duration = Duration::from_secs(60);
const DISK_BYTES: usize = 12 * 1024 * 1024;
const SLICE_START_LBA: u32 = 2048;
const SLICE_SECTORS: u32 = 20_160;
const MBR_SIGNATURE_OFFSET: usize = 510;
const MBR_PARTITION_TABLE_OFFSET: usize = 446;
const MBR_ENTRY_BYTES: usize = 16;
const GREETING: &[u8] = b"greetings from the FAT root\n";
const MENU: &[u8] = b"boot files live under /boot\n";

fn skip_reason() -> Option<&'static str> {
    if Arch::current() != Arch::X86_32 {
        return Some("the FAT boot disk flow belongs to the x86_32 port");
    }
    if Command::new("mformat")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_err()
    {
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

fn host_run(program: &str, args: &[&str]) {
    let status = Command::new(program)
        .args(args)
        .status()
        .unwrap_or_else(|error| panic!("{program} unavailable: {error}"));
    assert!(status.success(), "{program} {args:?} failed");
}

fn stamp_mbr_entry(image: &mut [u8], slot: usize, type_id: u8, start_lba: u32, sector_count: u32) {
    let entry =
        &mut image[MBR_PARTITION_TABLE_OFFSET + slot * MBR_ENTRY_BYTES..][..MBR_ENTRY_BYTES];
    entry[0] = if slot == 0 { 0x80 } else { 0x00 };
    entry[4] = type_id;
    entry[8..12].copy_from_slice(&start_lba.to_le_bytes());
    entry[12..16].copy_from_slice(&sector_count.to_le_bytes());
}

fn slice_offset_argument(disk: &Path) -> String {
    let image = disk.to_str().expect("utf8 image path");
    format!("{image}@@{}", SLICE_START_LBA as usize * 512)
}

fn write_seed(disk: &Path, name: &str, payload: &[u8], target: &str) {
    let seed = disk.with_extension(name);
    std::fs::write(&seed, payload).expect("write the seed fixture");
    let destination = format!("::{name}");
    host_run(
        "mcopy",
        &["-m", "-i", target, seed.to_str().expect("utf8"), &destination],
    );
    std::fs::remove_file(&seed).expect("drop the seed fixture");
}

fn build_boot_disk(disk: &Path, with_shell: bool) {
    std::fs::write(disk, vec![0u8; DISK_BYTES]).expect("create the boot disk image");
    {
        let mut image = std::fs::read(disk).expect("read the boot disk image");
        stamp_mbr_entry(&mut image, 0, 0x06, SLICE_START_LBA, SLICE_SECTORS);
        image[MBR_SIGNATURE_OFFSET] = 0x55;
        image[MBR_SIGNATURE_OFFSET + 1] = 0xAA;
        std::fs::write(disk, image).expect("stamp the mbr");
    }
    let target = slice_offset_argument(disk);
    host_run(
        "mformat",
        &[
            "-i", &target, "-h", "16", "-s", "63", "-T", &SLICE_SECTORS.to_string(), "-r", "224",
            "-c", "1", "-v", "ROSBOOT",
        ],
    );
    host_run("mmd", &["-i", &target, "::/boot"]);
    write_seed(disk, "greeting.txt", GREETING, &target);
    let menu = disk.with_extension("menu.txt");
    std::fs::write(&menu, MENU).expect("write the menu fixture");
    host_run("mcopy", &["-m", "-i", &target, menu.to_str().expect("utf8"), "::/boot/menu.txt"]);
    std::fs::remove_file(&menu).expect("drop the menu fixture");
    if with_shell {
        for app in ["shell", "mkdir"] {
            let elf = user_elf(app);
            let destination = format!("::{app}");
            host_run("mcopy", &["-m", "-i", &target, elf.to_str().expect("utf8"), &destination]);
        }
    }
}

fn user_elf(app: &str) -> PathBuf {
    let elf = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("target")
        .join(Arch::current().user_dir_name())
        .join("release")
        .join(app);
    if !elf.is_file() {
        panic!(
            "missing user ELF {}; run `cargo xtask build {}` first",
            elf.display(),
            Arch::current().name()
        );
    }
    elf
}

#[test]
fn boot_disk_mounts_the_root_and_hands_off_the_disk_shell() {
    if let Some(reason) = skip_reason() {
        eprintln!("skipped: {reason}");
        return;
    }
    let staging = tempfile::tempdir().expect("tempdir for the boot disk image");
    let disk = staging.path().join("boot-disk.img");
    build_boot_disk(&disk, true);

    let mut session = QemuSession::spawn_with_master_disk(&disk);
    session.expect_output("attach registered ATA", BOOT_BUDGET);
    session.expect_output("mount ok device=ata0d0p0 dir=/", BOOT_BUDGET);
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

#[test]
fn a_mounted_root_without_a_shell_falls_back_to_the_rom_shell() {
    if let Some(reason) = skip_reason() {
        eprintln!("skipped: {reason}");
        return;
    }
    let staging = tempfile::tempdir().expect("tempdir for the boot disk image");
    let disk = staging.path().join("boot-disk.img");
    build_boot_disk(&disk, false);

    let mut session = QemuSession::spawn_with_master_disk(&disk);
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
