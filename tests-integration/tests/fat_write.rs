use fat_image::{FatDisk, FatKind};
use std::process::Command;
use std::time::Duration;
use tests_integration::kernel_build::user_elf;
use tests_integration::qemu::QemuSession;

const BOOT_BUDGET: Duration = Duration::from_secs(90);
const OP_BUDGET: Duration = Duration::from_secs(120);
const FAT_BYTES: u64 = 50 * 1024 * 1024;

fn run(session: &mut QemuSession, command: &str, needle: &str) -> String {
    let mark = session.mark();
    session.send_line(command);
    session.expect_output_since(mark, needle, OP_BUDGET);
    if needle != "rose>" {
        let after = session.mark();
        session.expect_output_since(after, "rose>", OP_BUDGET);
    }
    session.content_since(mark)
}

fn boot_with_fat(disk: &std::path::Path) -> QemuSession {
    let mut session = QemuSession::spawn_with_hdd_disk(disk);
    session.expect_output("[VFSD] automounted hdd0 at /hdd", BOOT_BUDGET);
    session.expect_output("rose>", BOOT_BUDGET);
    session
}

fn mtools_missing() -> bool {
    Command::new("mformat")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_err()
}

fn mdir(disk: &std::path::Path, target: &str) -> bool {
    let status = Command::new("mdir")
        .args(["-i", disk.to_str().expect("utf8 image path"), target])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .expect("mdir");
    status.success()
}

fn mcopy_out(disk: &std::path::Path, entry: &str, out: &std::path::Path) {
    let status = Command::new("mcopy")
        .args([
            "-i",
            disk.to_str().expect("utf8 image path"),
            &format!("::{entry}"),
            out.to_str().expect("utf8 out path"),
        ])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .expect("mcopy");
    assert!(status.success(), "mcopy {entry} must succeed");
}

#[test]
fn files_written_by_the_guest_read_back_on_a_standard_system() {
    let volume_dir = tempfile::tempdir().expect("tempdir for the fat image");
    let disk = volume_dir.path().join("fat-write-50mb.img");
    let mut image = FatDisk::new(FatKind::Fat16, FAT_BYTES).expect("plan the fat volume");
    let mem = std::fs::read(user_elf("mem")).expect("read the mem user ELF");
    image.add_file("MEM", &mem).expect("pack mem into the fat volume");
    image.write_to(&disk).expect("write the fat image");

    {
        let mut session = boot_with_fat(&disk);
        let listing = run(&mut session, "ls /hdd", "rose>");
        assert!(listing.contains("mem"), "the packed file must list: {listing}");

        run(&mut session, "cp mem hdd/mem2", "cp ok mem -> hdd/mem2");
        run(&mut session, "mkdir hdd/sub", "mkdir ok hdd/sub");
        run(&mut session, "cp echo hdd/sub/e", "cp ok echo -> hdd/sub/e");
        run(&mut session, "rm hdd/mem", "rm ok hdd/mem");

        let listing = run(&mut session, "ls /hdd", "rose>");
        assert!(listing.contains("mem2"), "the written file must list: {listing}");
        assert!(
            !listing.contains("mem\t") && !listing.contains("mem\n"),
            "the removed file must vanish: {listing}"
        );
        let listing = run(&mut session, "ls hdd/sub", "rose>");
        assert!(listing.contains("e"), "the nested copy must list: {listing}");
    }

    {
        let mut session = boot_with_fat(&disk);
        let listing = run(&mut session, "ls /hdd", "rose>");
        assert!(
            listing.contains("mem2") && listing.contains("sub"),
            "guest-written files must survive the reboot: {listing}"
        );
        assert!(
            !listing.contains("mem\t") && !listing.contains("mem\n"),
            "the removal must survive the reboot: {listing}"
        );
        let listing = run(&mut session, "ls hdd/sub", "rose>");
        assert!(listing.contains("e"), "nested content must survive: {listing}");

        run(&mut session, "rm hdd/sub/e", "rm ok hdd/sub/e");
        run(&mut session, "rmdir hdd/sub", "rmdir ok hdd/sub");
    }

    if mtools_missing() {
        eprintln!("note: mtools not installed, skipping the host-side byte proof");
        return;
    }
    assert!(mdir(&disk, "::MEM2"), "mem2 must be a standard-readable file");
    assert!(!mdir(&disk, "::MEM"), "the deleted file must be gone for mtools");
    assert!(!mdir(&disk, "::SUB"), "the removed directory must be gone for mtools");
    let extracted = volume_dir.path().join("extracted-mem2");
    mcopy_out(&disk, "MEM2", &extracted);
    assert_eq!(
        std::fs::read(&extracted).expect("extracted bytes"),
        mem,
        "the guest-written file must match the source ELF byte for byte"
    );
}
