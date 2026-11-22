use std::time::Duration;
use tests_integration::qemu::QemuSession;
use tests_integration::shared::SharedSession;

const BOOT_TIMEOUT: Duration = Duration::from_secs(90);
const EXIT_TIMEOUT: Duration = Duration::from_secs(15);
const SHELL_TIMEOUT: Duration = Duration::from_secs(10);

#[test]
fn boot_shell_and_exit_status_share_one_session() {
    let mut group = SharedSession::spawn("boot_and_exit_status");

    group.case("kernel_boots", kernel_boots);
    group.case("shell_banner_and_prompt", shell_banner_and_prompt);
    group.case("nic_less_boot_reports_the_exited_network_service", nic_less_boot_reports_the_exited_network_service);
    group.case("shell_echoes_keystrokes", shell_echoes_keystrokes);
    group.case("shell_ls_and_unknown_command", shell_ls_and_unknown_command);
    group.case("exiting_from_main_is_silently_clean", exiting_from_main_is_silently_clean);
    group.case("explicit_exit_reports_its_code_to_the_shell", explicit_exit_reports_its_code_to_the_shell);
    group.case("a_waiting_parent_always_receives_the_clean_seven", a_waiting_parent_always_receives_the_clean_seven);
    group.case("user_panic_reports_an_abnormal_exit_to_the_shell", user_panic_reports_an_abnormal_exit_to_the_shell);
    group.case("malformed_exit_request_reports_usage_and_the_shell_survives", malformed_exit_request_reports_usage_and_the_shell_survives);
}

fn kernel_boots(session: &mut QemuSession) {
    session.expect_output("[KERNEL] Initializing", BOOT_TIMEOUT);
    session.expect_output("[KERNEL] Starting", BOOT_TIMEOUT);
}

fn shell_banner_and_prompt(session: &mut QemuSession) {
    session.expect_output("ROSE Shell", Duration::from_secs(60));
    session.expect_output("rose>", Duration::from_secs(60));
}

fn nic_less_boot_reports_the_exited_network_service(session: &mut QemuSession) {
    session.expect_output("[INIT] NE2K service exited before ready", Duration::from_secs(60));
    session.expect_output("rose>", Duration::from_secs(60));
}

fn shell_echoes_keystrokes(session: &mut QemuSession) {
    let mark = session.mark();
    session.send_text("hello");
    session.expect_output_since(mark, "hello", SHELL_TIMEOUT);
}

fn shell_ls_and_unknown_command(session: &mut QemuSession) {
    let after_prompt = session.mark();
    session.send_line("ls");
    session.expect_output_since(after_prompt, "clear", SHELL_TIMEOUT);
    session.expect_output_since(after_prompt, "tetris", SHELL_TIMEOUT);
    session.expect_output_since(after_prompt, "rose>", SHELL_TIMEOUT);
    let after_ls = session.mark();
    session.send_line("foo");
    session.expect_output_since(after_ls, "Unknown command: foo", SHELL_TIMEOUT);
}

fn exiting_from_main_is_silently_clean(session: &mut QemuSession) {
    let after_bare = session.mark();
    session.send_line("exiter");
    session.expect_output_since(after_bare, "rose>", EXIT_TIMEOUT);
    let report = session.content_since(after_bare);
    assert!(
        !report.contains("exited") && !report.contains("panicked"),
        "a clean zero exit must not produce a report, got:\n{report}"
    );
}

fn explicit_exit_reports_its_code_to_the_shell(session: &mut QemuSession) {
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

fn a_waiting_parent_always_receives_the_clean_seven(session: &mut QemuSession) {
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

fn user_panic_reports_an_abnormal_exit_to_the_shell(session: &mut QemuSession) {
    let after_panic = session.mark();
    session.send_line("exiter panic");
    session.expect_output_since(after_panic, "exiter panicked", EXIT_TIMEOUT);
    session.expect_output_since(after_panic, "rose>", EXIT_TIMEOUT);
}

fn malformed_exit_request_reports_usage_and_the_shell_survives(session: &mut QemuSession) {
    let after_usage = session.mark();
    session.send_line("exiter not-a-code");
    session.expect_output_since(after_usage, "usage: exiter [code|panic]", EXIT_TIMEOUT);
    session.expect_output_since(after_usage, "exiter exited with code 2", EXIT_TIMEOUT);
    session.expect_output_since(after_usage, "rose>", EXIT_TIMEOUT);
}
