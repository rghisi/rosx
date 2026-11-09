use std::time::Duration;
use tests_integration::qemu::QemuSession;

const BOOT_TIMEOUT: Duration = Duration::from_secs(90);
const EXIT_TIMEOUT: Duration = Duration::from_secs(15);

#[test]
fn exiting_from_main_is_silently_clean() {
    let mut session = QemuSession::spawn();
    session.expect_output("rose>", BOOT_TIMEOUT);

    let after_bare = session.mark();
    session.send_line("exiter");
    session.expect_output_since(after_bare, "rose>", EXIT_TIMEOUT);
    let report = session.content_since(after_bare);
    assert!(
        !report.contains("exited") && !report.contains("panicked"),
        "a clean zero exit must not produce a report, got:\n{report}"
    );
}

#[test]
fn explicit_exit_reports_its_code_to_the_shell() {
    let mut session = QemuSession::spawn();
    session.expect_output("rose>", BOOT_TIMEOUT);

    let after_zero = session.mark();
    session.send_line("exiter 0");
    session.expect_output_since(after_zero, "rose>", EXIT_TIMEOUT);
    assert!(
        !session.content_since(after_zero).contains("exited"),
        "exit 0 is a clean zero and must not produce a report"
    );

    let after_seven = session.mark();
    session.send_line("exiter 7");
    session.expect_output_since(after_seven, "exiter exited with code 7", EXIT_TIMEOUT);
    session.expect_output_since(after_seven, "rose>", EXIT_TIMEOUT);
}

#[test]
fn a_waiting_parent_always_receives_the_clean_seven() {
    let mut session = QemuSession::spawn();
    session.expect_output("rose>", BOOT_TIMEOUT);

    for round in 1..=10 {
        let after = session.mark();
        session.send_line("exiter 7");
        session.expect_output_since(after, "exiter exited with code 7", EXIT_TIMEOUT);
        session.expect_output_since(after, "rose>", EXIT_TIMEOUT);
        let report = session.content_since(after);
        assert_eq!(
            report.matches("exited with").count(),
            1,
            "round {round}: the waiting parent must receive Clean(7) exactly once, got:\n{report}"
        );
    }
}

#[test]
fn user_panic_reports_an_abnormal_exit_to_the_shell() {
    let mut session = QemuSession::spawn();
    session.expect_output("rose>", BOOT_TIMEOUT);

    let after_panic = session.mark();
    session.send_line("exiter panic");
    session.expect_output_since(after_panic, "exiter panicked", EXIT_TIMEOUT);
    session.expect_output_since(after_panic, "rose>", EXIT_TIMEOUT);
}

#[test]
fn malformed_exit_request_reports_usage_and_the_shell_survives() {
    let mut session = QemuSession::spawn();
    session.expect_output("rose>", BOOT_TIMEOUT);

    let after_usage = session.mark();
    session.send_line("exiter not-a-code");
    session.expect_output_since(after_usage, "usage: exiter [code|panic]", EXIT_TIMEOUT);
    session.expect_output_since(after_usage, "exiter exited with code 2", EXIT_TIMEOUT);
    session.expect_output_since(after_usage, "rose>", EXIT_TIMEOUT);
}
