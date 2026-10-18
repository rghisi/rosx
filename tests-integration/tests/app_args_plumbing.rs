use std::time::Duration;
use tests_integration::qemu::QemuSession;

#[test]
fn shell_launches_converted_app_with_arguments() {
    let mut session = QemuSession::spawn();
    session.expect_output("rose>", Duration::from_secs(60));

    let mark = session.mark();
    session.send_line("pi");
    session.expect_output_since(mark, "Pi:", Duration::from_secs(10));
}

#[test]
fn unknown_command_with_extra_args_is_rejected() {
    let mut session = QemuSession::spawn();
    session.expect_output("rose>", Duration::from_secs(60));

    let mark = session.mark();
    session.send_line("foo 123");
    session.expect_output_since(mark, "Unknown command: foo", Duration::from_secs(10));
}
