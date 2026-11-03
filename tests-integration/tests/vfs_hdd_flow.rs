use std::time::Duration;
use tests_integration::qemu::QemuSession;

const BOOT_BUDGET: Duration = Duration::from_secs(90);
const OP_BUDGET: Duration = Duration::from_secs(60);
const HDD_BYTES: usize = 50 * 1024 * 1024;

#[test]
fn format_mount_and_remount_the_hard_disk_volume() {
    let volume_dir = tempfile::tempdir().expect("tempdir for the hdd image");
    let disk = volume_dir.path().join("hdd-50mb.img");
    std::fs::write(&disk, vec![0u8; HDD_BYTES]).expect("create the hdd image");

    {
        let mut session = QemuSession::spawn_with_hdd_disk(&disk);
        session.expect_output("[ATAD] ata0 drive1 ready:", BOOT_BUDGET);
        session.expect_output("rose>", BOOT_BUDGET);

        let mark = session.mark();
        session.send_line("ata list");
        session.expect_output_since(mark, "ata disk 0", OP_BUDGET);
        session.expect_output_since(mark, "ata disk 1 sectors=102400", OP_BUDGET);

        let mark = session.mark();
        session.send_line("mount 1 /hdd");
        session.expect_output_since(mark, "mount failed: Vfs(NotFormatted)", OP_BUDGET);

        let mark = session.mark();
        session.send_line("format 1");
        session.expect_output_since(mark, "format ok drive=1", OP_BUDGET);

        let mark = session.mark();
        session.send_line("mount 1 /hdd");
        session.expect_output_since(mark, "mount ok drive=1 dir=/hdd", OP_BUDGET);

        let mark = session.mark();
        session.send_line("ls /hdd");
        session.expect_output_since(mark, "rose>", OP_BUDGET);
        let listing = session.content_since(mark);
        assert!(
            !listing.contains("Cannot list"),
            "ls /hdd on the fresh volume must succeed: {listing}"
        );
    }

    let mut session = QemuSession::spawn_with_hdd_disk(&disk);
    session.expect_output("rose>", BOOT_BUDGET);

    let mark = session.mark();
    session.send_line("mount 1 /hdd");
    session.expect_output_since(mark, "mount ok drive=1 dir=/hdd", OP_BUDGET);

    let mark = session.mark();
    session.send_line("ls /hdd");
    session.expect_output_since(mark, "rose>", OP_BUDGET);
    let listing = session.content_since(mark);
    assert!(
        !listing.contains("Cannot list"),
        "ls /hdd after remount must succeed: {listing}"
    );
}
