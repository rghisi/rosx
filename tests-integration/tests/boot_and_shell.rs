use std::time::Duration;
use tests_integration::qemu::QemuSession;

#[test]
fn kernel_boots() {
    let session = QemuSession::spawn();
    session.expect_output("[KERNEL] Initializing", Duration::from_secs(60));
    session.expect_output("[KERNEL] Starting", Duration::from_secs(60));
}

#[test]
fn shell_banner_and_prompt() {
    let session = QemuSession::spawn();
    session.expect_output("ROSE Shell", Duration::from_secs(60));
    session.expect_output("rose>", Duration::from_secs(60));
}

#[test]
fn shell_echoes_keystrokes() {
    let mut session = QemuSession::spawn();
    session.expect_output("rose>", Duration::from_secs(60));
    session.send_text("hello");
    session.expect_output("hello", Duration::from_secs(10));
}
