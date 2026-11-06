use std::time::Duration;
use fat_image::{FatDisk, FatKind};
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

#[test]
fn executes_a_program_loose_on_a_fat_volume() {
    let volume_dir = tempfile::tempdir().expect("tempdir for the fat image");
    let disk = volume_dir.path().join("fat-exec-50mb.img");
    let mut image = FatDisk::new(FatKind::Fat16, FAT_BYTES).expect("plan the fat volume");
    let mem = std::fs::read(user_elf("mem")).expect("read the mem user ELF");
    image.add_file("MEM", &mem).expect("pack mem into the fat volume");
    image.write_to(&disk).expect("write the fat image");

    let mut session = QemuSession::spawn_with_hdd_disk(&disk);
    session.expect_output("[VFSD] automounted hdd0 at /hdd", BOOT_BUDGET);
    session.expect_output("rose>", BOOT_BUDGET);

    run(&mut session, "ata list", "ata disk 1 sectors=102400");

    let listing = run(&mut session, "ls /hdd", "rose>");
    assert!(
        listing.contains("mem"),
        "the packed program must list on the auto-detected fat volume: {listing}"
    );

    let reply = run(&mut session, "format hdd0", "format failed: Vfs(Busy)");
    assert!(reply.contains("Busy"), "a mounted volume never reformats: {reply}");

    let mark = session.mark();
    session.send_line("hdd/mem");
    session.expect_output_since(mark, "KSTATSD memory snapshot", OP_BUDGET);
    session.expect_output_since(mark, "rose>", OP_BUDGET);

    let launched = session.content_since(mark);
    assert!(
        !launched.contains("Cannot launch") && !launched.contains("truncated"),
        "the program must exec from the fat volume: {launched}"
    );
}
