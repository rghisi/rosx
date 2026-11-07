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
fn attach_format_and_carve_directories_on_the_hard_disk_volume() {
    let volume_dir = tempfile::tempdir().expect("tempdir for the hdd image");
    let disk = volume_dir.path().join("hdd-50mb.img");
    std::fs::write(&disk, vec![0u8; HDD_BYTES]).expect("create the hdd image");

    {
        let mut session = QemuSession::spawn_with_hdd_disk(&disk);
        session.expect_output("[ATAD] ata0 drive1 ready:", BOOT_BUDGET);
        session.expect_output("rose>", BOOT_BUDGET);

        let listing = run(&mut session, "ata list", "ata disk 1 sectors=102400");
        assert!(listing.contains("ata disk 0"), "both drives must list: {listing}");

        let reply = run(&mut session, "mount ata0 /hdd", "mount failed: Vfs(NotFound)");
        assert!(reply.contains("NotFound"), "no mount point yet: {reply}");

        run(&mut session, "mkdir /hdd", "mkdir ok /hdd");

        let reply = run(&mut session, "mount ata0 /hdd", "mount failed: Vfs(NotFound)");
        assert!(reply.contains("NotFound"), "no device attached yet: {reply}");

        run(&mut session, "attach ata0 ATA 1", "attach ok id=ata0 service=ATA device=1");
        run(&mut session, "attach list", "ata0 provider ATA drive 1 blocks=102400 size=512 free");

        run(&mut session, "mount ata0 /hdd", "mount failed: Vfs(NotFormatted)");
        run(&mut session, "format ata0", "format ok device=ata0");
        run(&mut session, "mount ata0 /hdd", "mount ok device=ata0 dir=/hdd");

        run(&mut session, "attach list", "ata0 provider ATA drive 1 blocks=102400 size=512 mounted at /hdd");

        let listing = run(&mut session, "ls /hdd", "rose>");
        assert!(
            !listing.contains("Cannot list"),
            "ls /hdd on the fresh volume must succeed: {listing}"
        );

        run(&mut session, "cp /pi /hdd/pi", "cp ok /pi -> /hdd/pi");

        let listing = run(&mut session, "ls /hdd", "rose>");
        assert!(listing.contains("pi"), "the copied file must list: {listing}");

        run(&mut session, "mkdir /hdd/sub", "mkdir ok /hdd/sub");
        run(&mut session, "cp /echo /hdd/sub/echo", "cp ok /echo -> /hdd/sub/echo");

        let listing = run(&mut session, "ls /hdd/sub", "rose>");
        assert!(listing.contains("echo"), "the nested copy must list: {listing}");

        run(&mut session, "rm /hdd", "rm failed: Vfs(NotPermitted)");

        let reply = run(&mut session, "attach ata1 ATA 1", "attach failed: Vfs(Busy)");
        assert!(reply.contains("Busy"), "one drive never attaches twice: {reply}");
        let reply = run(&mut session, "format ata0", "format failed: Vfs(Busy)");
        assert!(reply.contains("Busy"), "a mounted drive never reformats: {reply}");
    }

    let mut session = QemuSession::spawn_with_hdd_disk(&disk);
    session.expect_output("rose>", BOOT_BUDGET);

    run(&mut session, "mkdir /hdd", "mkdir ok /hdd");
    run(&mut session, "attach ata0 ATA 1", "attach ok id=ata0 service=ATA device=1");
    run(&mut session, "mount ata0 /hdd", "mount ok device=ata0 dir=/hdd");

    let listing = run(&mut session, "ls /hdd", "rose>");
    assert!(
        listing.contains("pi") && listing.contains("sub"),
        "the disk must survive the reboot: {listing}"
    );

    let listing = run(&mut session, "ls /hdd/sub", "rose>");
    assert!(listing.contains("echo"), "nested content must survive: {listing}");

    run(&mut session, "rm /hdd/pi", "rm ok /hdd/pi");

    let listing = run(&mut session, "ls /hdd", "rose>");
    assert!(!listing.contains("pi\t"), "rm must clear the file: {listing}");

    run(&mut session, "rmdir /hdd/sub", "rmdir failed: Vfs(DirectoryNotEmpty)");
    run(&mut session, "rm /hdd/sub/echo", "rm ok /hdd/sub/echo");
    run(&mut session, "rmdir /hdd/sub", "rmdir ok /hdd/sub");
}
