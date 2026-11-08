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
fn a_corrupt_partition_table_exposes_the_whole_drive_for_rescue() {
    let volume_dir = tempfile::tempdir().expect("tempdir for the corrupt image");
    let disk = volume_dir.path().join("hdd-corrupt.img");
    std::fs::write(&disk, vec![0u8; HDD_BYTES]).expect("create the hdd image");

    let mut session = QemuSession::spawn_with_corrupt_partitioned_hdd_disk(&disk);
    session.expect_output("[ATAD] ata0 drive1 ready:", BOOT_BUDGET);
    session.expect_output("[BLOCKD] ready", BOOT_BUDGET);
    session.expect_output("rose>", BOOT_BUDGET);

    let listing = run(&mut session, "attach ATA", "attach registered ATA");
    assert!(
        listing.contains("ata0d1  ") && listing.contains("corrupt"),
        "the corrupt drive must be listed whole with its honest state: {listing}"
    );
    assert!(
        !listing.contains("ata0d1p"),
        "a corrupt table must never produce slices: {listing}"
    );

    run(&mut session, "mkdir /data", "mkdir ok /data");
    run(&mut session, "format ata0d1", "format ok device=ata0d1");
    run(&mut session, "mount ata0d1 /data", "mount ok device=ata0d1 dir=/data");

    run(&mut session, "cp /pi /data/pi", "cp ok /pi -> /data/pi");
    let listing = run(&mut session, "ls /data", "rose>");
    assert!(listing.contains("pi"), "the rescue path must roundtrip: {listing}");

    let reply = run(&mut session, "format ata0d1p0", "format failed: Vfs(NotFound)");
    assert!(
        reply.contains("NotFound"),
        "a phantom slice must be refused with NotFound: {reply}"
    );
}
