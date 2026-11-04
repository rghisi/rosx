use std::time::Duration;
use tests_integration::qemu::QemuSession;

const BOOT_BUDGET: Duration = Duration::from_secs(90);
const OP_BUDGET: Duration = Duration::from_secs(60);
const HDD_BYTES: usize = 50 * 1024 * 1024;

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
fn mkdir_mount_and_carve_directories_on_the_hard_disk_volume() {
    let volume_dir = tempfile::tempdir().expect("tempdir for the hdd image");
    let disk = volume_dir.path().join("hdd-50mb.img");
    std::fs::write(&disk, vec![0u8; HDD_BYTES]).expect("create the hdd image");

    {
        let mut session = QemuSession::spawn_with_hdd_disk(&disk);
        session.expect_output("[ATAD] ata0 drive1 ready:", BOOT_BUDGET);
        session.expect_output("rose>", BOOT_BUDGET);

        let listing = run(&mut session, "ata list", "ata disk 1 sectors=102400");
        assert!(listing.contains("ata disk 0"), "both drives must list: {listing}");

        let reply = run(&mut session, "mount 1 /hdd", "mount failed: Vfs(NotFound)");
        assert!(reply.contains("NotFound"), "no mount point yet: {reply}");

        run(&mut session, "mkdir /hdd", "mkdir ok /hdd");

        run(&mut session, "mount 1 /hdd", "mount failed: Vfs(NotFormatted)");
        run(&mut session, "format 1", "format ok drive=1");
        run(&mut session, "mount 1 /hdd", "mount ok drive=1 dir=/hdd");

        let listing = run(&mut session, "ls /hdd", "rose>");
        assert!(
            !listing.contains("Cannot list"),
            "ls /hdd on the fresh volume must succeed: {listing}"
        );

        run(&mut session, "cp pi hdd/pi", "cp ok pi -> hdd/pi");

        let listing = run(&mut session, "ls /hdd", "rose>");
        assert!(listing.contains("pi"), "the copied file must list: {listing}");

        run(&mut session, "mkdir hdd/sub", "mkdir ok hdd/sub");
        run(&mut session, "cp echo hdd/sub/echo", "cp ok echo -> hdd/sub/echo");

        let listing = run(&mut session, "ls hdd/sub", "rose>");
        assert!(listing.contains("echo"), "the nested copy must list: {listing}");

        run(&mut session, "rm /hdd", "rm failed: Vfs(NotPermitted)");
    }

    let mut session = QemuSession::spawn_with_hdd_disk(&disk);
    session.expect_output("rose>", BOOT_BUDGET);

    run(&mut session, "mkdir /hdd", "mkdir ok /hdd");
    run(&mut session, "mount 1 /hdd", "mount ok drive=1 dir=/hdd");

    let listing = run(&mut session, "ls /hdd", "rose>");
    assert!(
        listing.contains("pi") && listing.contains("sub"),
        "the disk must survive the reboot: {listing}"
    );

    let listing = run(&mut session, "ls hdd/sub", "rose>");
    assert!(listing.contains("echo"), "nested content must survive: {listing}");

    run(&mut session, "rm hdd/pi", "rm ok hdd/pi");

    let listing = run(&mut session, "ls /hdd", "rose>");
    assert!(!listing.contains("pi\t"), "rm must clear the file: {listing}");

    run(&mut session, "rmdir hdd/sub", "rmdir failed: Vfs(DirectoryNotEmpty)");
    run(&mut session, "rm hdd/sub/echo", "rm ok hdd/sub/echo");
    run(&mut session, "rmdir hdd/sub", "rmdir ok hdd/sub");
}
