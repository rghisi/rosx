use std::time::Duration;
use tests_integration::qemu::QemuSession;
use tests_integration::shared::SharedSession;

const LAUNCH_TIMEOUT: Duration = Duration::from_secs(15);
const REAP_PASS_TIMEOUT: Duration = Duration::from_secs(10);
const REAP_PASSES: usize = 5;

// The shell reaps finished background jobs before every prompt, so a report can
// surface on a later launch line. `from` must therefore be a mark taken before
// the launches the reports belong to, never the helper's own entry point.
fn reap_pass_until(session: &mut QemuSession, from: usize, needle: &str) {
    for _pass in 1..=REAP_PASSES {
        session.send_key('\n');
        let arrived = session
            .wait_until_since(from, |content| content.contains(needle), REAP_PASS_TIMEOUT)
            .is_ok();
        if arrived {
            return;
        }
    }
    panic!(
        "no {needle:?} after {REAP_PASSES} prompt reaps:\n{}",
        session.content_since(from)
    );
}

#[test]
fn shell_background_cases_share_one_session() {
    let mut group = SharedSession::spawn("shell_background");
    group.case(
        "background_launch_announces_start_and_finishes_on_a_later_prompt",
        background_launch_announces_start_and_finishes_on_a_later_prompt,
    );
    group.case(
        "a_foreground_command_still_waits_while_a_background_job_runs",
        a_foreground_command_still_waits_while_a_background_job_runs,
    );
    group.case(
        "a_panicking_background_job_is_reported_at_a_later_prompt",
        a_panicking_background_job_is_reported_at_a_later_prompt,
    );
    group.case(
        "a_built_in_announces_that_the_background_request_is_ignored",
        a_built_in_announces_that_the_background_request_is_ignored,
    );
    group.case(
        "repeated_background_launches_are_each_reaped_exactly_once",
        repeated_background_launches_are_each_reaped_exactly_once,
    );
}

fn background_launch_announces_start_and_finishes_on_a_later_prompt(session: &mut QemuSession) {
    let from = session.mark();
    session.send_line("exiter 5 &");
    session.expect_output_since(from, "exiter started", LAUNCH_TIMEOUT);
    session.expect_output_since(from, "rose>", LAUNCH_TIMEOUT);
    assert!(
        !session.content_since(from).contains("exited"),
        "a background job must report nothing on its own launch line, got:\n{}",
        session.content_since(from)
    );

    reap_pass_until(session, from, "exiter exited with code 5");
}

fn a_foreground_command_still_waits_while_a_background_job_runs(session: &mut QemuSession) {
    let from = session.mark();
    session.send_line("exiter 5 &");
    session.expect_output_since(from, "exiter started", LAUNCH_TIMEOUT);
    session.expect_output_since(from, "rose>", LAUNCH_TIMEOUT);

    session.send_line("exiter 3");
    session.expect_output_since(from, "exiter exited with code 3", LAUNCH_TIMEOUT);
    session.expect_output_since(from, "rose>", LAUNCH_TIMEOUT);

    reap_pass_until(session, from, "exiter exited with code 5");
    let report = session.content_since(from);
    assert_eq!(report.matches("exited with code 3").count(), 1, "got:\n{report}");
    assert_eq!(report.matches("exited with code 5").count(), 1, "got:\n{report}");
}

fn a_panicking_background_job_is_reported_at_a_later_prompt(session: &mut QemuSession) {
    let from = session.mark();
    session.send_line("exiter panic &");
    session.expect_output_since(from, "exiter started", LAUNCH_TIMEOUT);
    session.expect_output_since(from, "rose>", LAUNCH_TIMEOUT);

    reap_pass_until(session, from, "exiter panicked");
}

fn a_built_in_announces_that_the_background_request_is_ignored(session: &mut QemuSession) {
    let after_builtin = session.mark();
    session.send_line("sleep &");
    session.expect_output_since(
        after_builtin,
        "Built-in runs in the foreground: sleep",
        LAUNCH_TIMEOUT,
    );
    session.expect_output_since(after_builtin, "rose>", LAUNCH_TIMEOUT);
}

fn repeated_background_launches_are_each_reaped_exactly_once(session: &mut QemuSession) {
    let from = session.mark();
    for code in 1..=3 {
        let after_launch = session.mark();
        session.send_line(&format!("exiter {code} &"));
        session.expect_output_since(after_launch, "exiter started", LAUNCH_TIMEOUT);
        session.expect_output_since(after_launch, "rose>", LAUNCH_TIMEOUT);
    }

    for code in 1..=3 {
        reap_pass_until(session, from, &format!("exiter exited with code {code}"));
    }
    let report = session.content_since(from);
    assert_eq!(
        report.matches("exiter started").count(),
        3,
        "every launch must announce itself exactly once, got:\n{report}"
    );
    for code in 1..=3 {
        assert_eq!(
            report.matches(&format!("exited with code {code}")).count(),
            1,
            "code {code} must be reaped exactly once, got:\n{report}"
        );
    }
}
