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
fn a_refresh_after_a_vendor_face_table_edit_recurates_the_namespace() {
    let volume_dir = tempfile::tempdir().expect("tempdir for the scratch image");
    let disk = volume_dir.path().join("hdd-rescan.img");
    std::fs::write(&disk, vec![0u8; HDD_BYTES]).expect("create the hdd image");

    let mut session = QemuSession::spawn_with_partitioned_hdd_disk(&disk);
    session.expect_output("[ATAD] ata0 drive1 ready:", BOOT_BUDGET);
    session.expect_output("[BLOCKD] ready", BOOT_BUDGET);
    session.expect_output("rose>", BOOT_BUDGET);

    let listing = run(&mut session, "attach ATA", "attach registered ATA");
    assert!(
        listing.contains("ata0d1p1  ") && !listing.contains("ata0d1  "),
        "the carved table must hide the drive before the edit: {listing}"
    );

    run(&mut session, "ata write 1 0", "writeback ok drive=1 lba=0");

    let listing = run(&mut session, "attach ATA", "attach registered ATA");
    assert!(
        listing.contains("ata0d1  ") && !listing.contains("ata0d1p"),
        "the refresh must re-read sector 0 and expose the drive whole again: {listing}"
    );
}
