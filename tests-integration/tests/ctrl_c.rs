use std::time::Duration;
use tests_integration::qemu::QemuSession;

const BOOT_TIMEOUT: Duration = Duration::from_secs(90);
const READY_TIMEOUT: Duration = Duration::from_secs(30);
const INTERRUPT_TIMEOUT: Duration = Duration::from_secs(20);
const FOLLOW_UP_TIMEOUT: Duration = Duration::from_secs(30);

#[test]
fn ctrl_c_kills_the_running_foreground_app() {
    let mut session = QemuSession::spawn();
    session.expect_output("rose>", BOOT_TIMEOUT);

    let mark = session.mark();
    session.send_line("spinner");
    session.expect_output_since(mark, "spinner ready", READY_TIMEOUT);

    session.send_key('\x03');

    session.expect_output_since(mark, "^C", INTERRUPT_TIMEOUT);
    session.expect_output_since(mark, "rose>", INTERRUPT_TIMEOUT);
    let window = session.content_since(mark);
    assert!(
        !window.contains("spinner done"),
        "the killed app must never reach its own ending line, got:\n{window}"
    );

    let after_kill = session.mark();
    session.send_line("exiter 7");
    session.expect_output_since(after_kill, "exiter exited with code 7", FOLLOW_UP_TIMEOUT);
    session.expect_output_since(after_kill, "rose>", FOLLOW_UP_TIMEOUT);
}

#[test]
fn ctrl_c_at_a_bare_prompt_redraws_and_keeps_the_shell_usable() {
    let mut session = QemuSession::spawn();
    session.expect_output("rose>", BOOT_TIMEOUT);

    let mark = session.mark();
    session.send_key('\x03');
    session.expect_output_since(mark, "^C", INTERRUPT_TIMEOUT);

    let after_prompt = session.mark();
    session.send_line("exiter 3");
    session.expect_output_since(after_prompt, "exiter exited with code 3", FOLLOW_UP_TIMEOUT);
    session.expect_output_since(after_prompt, "rose>", FOLLOW_UP_TIMEOUT);
}

#[test]
fn ctrl_c_after_a_partial_line_discards_the_line() {
    let mut session = QemuSession::spawn();
    session.expect_output("rose>", BOOT_TIMEOUT);

    let mark = session.mark();
    session.send_text("exiter pan");
    session.send_key('\x03');
    session.expect_output_since(mark, "^C", INTERRUPT_TIMEOUT);

    let after_interrupt = session.mark();
    session.send_line("exiter 4");
    session.expect_output_since(after_interrupt, "exiter exited with code 4", FOLLOW_UP_TIMEOUT);
    session.expect_output_since(after_interrupt, "rose>", FOLLOW_UP_TIMEOUT);
    let window = session.content_since(after_interrupt);
    assert!(
        !window.contains("panic"),
        "the discarded partial line must not execute, got:\n{window}"
    );
}

#[test]
fn ctrl_c_spares_background_jobs() {
    let mut session = QemuSession::spawn();
    session.expect_output("rose>", BOOT_TIMEOUT);

    let mark = session.mark();
    session.send_line("spinner &");
    session.expect_output_since(mark, "spinner started", READY_TIMEOUT);

    let victim_mark = session.mark();
    session.send_line("pi");
    session.expect_output_since(victim_mark, "Pi: ", READY_TIMEOUT);
    session.send_key('\x03');
    session.expect_output_since(victim_mark, "^C", INTERRUPT_TIMEOUT);
    session.expect_output_since(victim_mark, "rose>", INTERRUPT_TIMEOUT);

    session.expect_output_since(victim_mark, "spinner tick 20", FOLLOW_UP_TIMEOUT);
    let window = session.content_since(victim_mark);
    assert!(
        !window.contains("spinner finished")
            && !window.contains("spinner interrupted")
            && !window.contains("spinner exited"),
        "the background spinner must keep running through an unrelated interrupt, got:\n{window}"
    );
}
