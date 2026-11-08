use std::time::Duration;
use tests_integration::qemu::QemuSession;

const BOOT_BUDGET: Duration = Duration::from_secs(90);
const OP_BUDGET: Duration = Duration::from_secs(60);
const HDD_BYTES: usize = 50 * 1024 * 1024;

#[test]
fn blockd_boots_and_attach_registers_the_ata_provider() {
    let volume_dir = tempfile::tempdir().expect("tempdir for the hdd image");
    let disk = volume_dir.path().join("hdd-50mb.img");
    std::fs::write(&disk, vec![0u8; HDD_BYTES]).expect("create the hdd image");

    let mut session = QemuSession::spawn_with_hdd_disk(&disk);
    session.expect_output("[BLOCKD] block device manager", BOOT_BUDGET);
    session.expect_output("[BLOCKD] ready", BOOT_BUDGET);
    session.expect_output("rose>", BOOT_BUDGET);

    let mark = session.mark();
    session.send_line("attach ATA");
    session.expect_output_since(mark, "attach registered ATA", OP_BUDGET);
    session.expect_output_since(mark, "ata0d1", OP_BUDGET);
    session.expect_output_since(mark, "102400x512", OP_BUDGET);
    session.expect_output_since(mark, "rose>", OP_BUDGET);

    let second = session.mark();
    session.send_line("attach ATA");
    session.expect_output_since(second, "attach registered ATA", OP_BUDGET);
    session.expect_output_since(second, "rose>", OP_BUDGET);
}
