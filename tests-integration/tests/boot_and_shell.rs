use std::time::Duration;
use tests_integration::qemu::QemuSession;

#[test]
fn kernel_boots() {
    let session = QemuSession::spawn();
    session.expect_output("[KERNEL] Initializing", Duration::from_secs(60));
    session.expect_output("[KERNEL] Starting", Duration::from_secs(60));
}
