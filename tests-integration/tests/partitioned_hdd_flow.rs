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
fn slices_are_first_class_devices_on_a_partitioned_disk() {
    let volume_dir = tempfile::tempdir().expect("tempdir for the partitioned image");
    let disk = volume_dir.path().join("hdd-partitioned.img");
    std::fs::write(&disk, vec![0u8; HDD_BYTES]).expect("create the hdd image");

    {
        let mut session = QemuSession::spawn_with_partitioned_hdd_disk(&disk);
        session.expect_output("[ATAD] ata0 drive1 ready:", BOOT_BUDGET);
        session.expect_output("[BLOCKD] ready", BOOT_BUDGET);
        session.expect_output("rose>", BOOT_BUDGET);

        let listing = run(&mut session, "attach ATA", "attach registered ATA");
        assert!(
            listing.contains("ata0d1p1  ") && listing.contains("ata0d1p2  "),
            "both carved slices must be listed: {listing}"
        );
        assert!(
            listing.contains("ata0d1 type 83") && listing.contains("65536x512"),
            "the p1 line must carry the drive name, type byte and geometry: {listing}"
        );
        assert!(
            listing.contains("ata0d1 type 82") && listing.contains("4096x512"),
            "the p2 line must carry the drive name, type byte and geometry: {listing}"
        );
        assert!(
            !listing.contains("ata0d1  "),
            "the table wins: the whole drive must not exist in the namespace: {listing}"
        );

        run(&mut session, "mkdir /data", "mkdir ok /data");

        let reply = run(&mut session, "mount ata0d1 /data", "mount failed: Vfs(NotFound)");
        assert!(
            reply.contains("NotFound"),
            "the hidden drive must be unaddressable even with a live mount point: {reply}"
        );

        run(&mut session, "format ata0d1p1", "format ok device=ata0d1p1");
        run(&mut session, "mount ata0d1p1 /data", "mount ok device=ata0d1p1 dir=/data");

        run(&mut session, "cp /pi /data/pi", "cp ok /pi -> /data/pi");
        run(&mut session, "cp /echo /data/echo", "cp ok /echo -> /data/echo");

        let listing = run(&mut session, "ls /data", "rose>");
        assert!(
            listing.contains("pi") && listing.contains("echo"),
            "both copies must list on the slice volume: {listing}"
        );

        run(&mut session, "rm /data/echo", "rm ok /data/echo");
        let listing = run(&mut session, "ls /data", "rose>");
        assert!(
            listing.contains("pi") && !listing.contains("echo\t"),
            "rm must clear the file on the slice volume: {listing}"
        );
    }

    let mut session = QemuSession::spawn_with_partitioned_hdd_disk(&disk);
    session.expect_output("rose>", BOOT_BUDGET);

    let listing = run(&mut session, "attach ATA", "attach registered ATA");
    assert!(
        listing.contains("ata0d1p1  ") && !listing.contains("ata0d1  "),
        "the second boot must curate the same table the same way: {listing}"
    );

    run(&mut session, "mkdir /data", "mkdir ok /data");
    run(&mut session, "mount ata0d1p1 /data", "mount ok device=ata0d1p1 dir=/data");

    let listing = run(&mut session, "ls /data", "rose>");
    assert!(
        listing.contains("pi") && !listing.contains("echo\t"),
        "the slice volume must survive the reboot: {listing}"
    );

    let reply = run(&mut session, "format ata0d1p1", "format failed: Vfs(Busy)");
    assert!(reply.contains("Busy"), "a mounted slice never reformats: {reply}");
}
