use std::time::Duration;
use tests_integration::qemu::QemuSession;

const BOOT_BUDGET: Duration = Duration::from_secs(90);

#[test]
fn atad_reports_the_boot_disk_identity_at_boot() {
    if tests_integration::arch::not_ported_to_x86_32() {
        return;
    }
    let session = QemuSession::spawn();
    session.expect_output("[ATAD] ATA disk driver", BOOT_BUDGET);
    session.expect_output("[ATAD] ata0 drive0 ready:", BOOT_BUDGET);
    session.expect_output("sectors=", BOOT_BUDGET);
    session.expect_output("rose>", BOOT_BUDGET);
}
