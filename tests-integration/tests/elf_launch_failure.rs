use std::path::Path;
use std::time::Duration;
use tests_integration::fat_disk::{create_boot_master, tools_available, FatDisk};
use tests_integration::kernel_build::kernel_elf;
use tests_integration::qemu::QemuSession;

const BOOT_BUDGET: Duration = Duration::from_secs(120);
const LAUNCH_BUDGET: Duration = Duration::from_secs(60);
const NOTES: &[u8] = b"these are plain notes, not an executable\n";
const CORRUPT_ELF: &[u8] = b"\x7fELF and then nothing a loader could accept\n";

fn skip_reason() -> Option<&'static str> {
    if !tools_available() {
        return Some("mtools not installed, skipping the launched-blob flow");
    }
    None
}

fn seeded_boot_disk(staging: &Path) -> FatDisk {
    let disk = create_boot_master(staging, &kernel_elf());
    disk.write_seed(NOTES, "::notes.txt");
    disk.write_seed(CORRUPT_ELF, "::corrupt.elf");
    disk
}

fn spawn_seeded_session(staging: &Path) -> QemuSession {
    let disk = seeded_boot_disk(staging);
    let session = QemuSession::spawn_with_master_disk(disk.path());
    session.expect_output("rose>", BOOT_BUDGET);
    session
}

fn proves_the_system_is_up(session: &mut QemuSession, code: &str) {
    let after_failure = session.mark();
    session.send_line(&format!("exiter {code}"));
    session.expect_output_since(after_failure, &format!("exiter exited with code {code}"), LAUNCH_BUDGET);
    session.expect_output_since(after_failure, "rose>", LAUNCH_BUDGET);
}

#[test]
fn launching_a_text_file_reports_a_failed_launch_and_the_system_stays_up() {
    if let Some(reason) = skip_reason() {
        eprintln!("skipped: {reason}");
        return;
    }
    let staging = tempfile::tempdir().expect("tempdir for the seeded boot disk");
    let mut session = spawn_seeded_session(staging.path());

    let mark = session.mark();
    session.send_line("notes.txt");
    session.expect_output_since(mark, "Not an executable: notes.txt", LAUNCH_BUDGET);
    session.expect_output_since(mark, "rose>", LAUNCH_BUDGET);
    let window = session.content_since(mark);
    assert!(
        !window.to_lowercase().contains("panic"),
        "a text file must not destabilise the kernel, got:\n{window}"
    );

    proves_the_system_is_up(&mut session, "5");
}

#[test]
fn launching_a_corrupt_elf_fails_at_the_syscall_with_a_kernel_diagnostic() {
    if let Some(reason) = skip_reason() {
        eprintln!("skipped: {reason}");
        return;
    }
    let staging = tempfile::tempdir().expect("tempdir for the seeded boot disk");
    let mut session = spawn_seeded_session(staging.path());

    let mark = session.mark();
    session.send_line("corrupt.elf");
    session.expect_output_since(mark, "[ELF] 'corrupt.elf' rejected", LAUNCH_BUDGET);
    session.expect_output_since(mark, "Cannot launch: corrupt.elf", LAUNCH_BUDGET);
    session.expect_output_since(mark, "rose>", LAUNCH_BUDGET);
    let window = session.content_since(mark);
    assert!(
        !window.to_lowercase().contains("panic"),
        "a corrupt ELF must fail LoadElf, not the kernel, got:\n{window}"
    );

    proves_the_system_is_up(&mut session, "6");
}
