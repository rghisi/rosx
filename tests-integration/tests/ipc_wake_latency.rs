use std::time::Duration;
use tests_integration::qemu::QemuSession;

const BOOT_BUDGET: Duration = Duration::from_secs(90);
const BURST_BUDGET: Duration = Duration::from_secs(60);
const BEGIN_MARKER: &str = "WAKE BURST BEGIN";
const SENT_MARKER: &str = "WAKE BURST SENT";
const PONG_LINE: &str = "PONG 64 matched ok";

fn position_of(content: &str, needle: &str) -> usize {
    content.find(needle).unwrap_or_else(|| panic!("missing {needle:?} in session content:\n{content}"))
}

#[test]
fn parked_services_run_before_the_waking_burst_completes() {
    let mut session = QemuSession::spawn();
    session.expect_output("[IPC Server] Ping", BOOT_BUDGET);
    session.expect_output("[IPC Server] Random", BOOT_BUDGET);
    session.expect_output("rose>", BOOT_BUDGET);

    let mark = session.mark();
    session.send_line("ping burst");
    if session
        .wait_until_since(mark, |delta| delta.contains(SENT_MARKER), BURST_BUDGET)
        .is_err()
    {
        panic!(
            "burst never completed within {BURST_BUDGET:?}\ncaptured content:\n{}",
            session.content_since(mark),
        );
    }

    let _ = session.wait_until_since(mark, |delta| delta.contains(PONG_LINE), Duration::from_secs(5));
    let content = session.content_since(mark);
    let begin = position_of(&content, BEGIN_MARKER);
    let sent = position_of(&content, SENT_MARKER);
    let pong = position_of(&content, PONG_LINE);

    assert!(
        pong > begin && pong < sent,
        "pong service ran only after the producer finished: a wake does not reschedule the waker\npong at {pong}, begin at {begin}, sent at {sent}\ncaptured content:\n{content}"
    );
    for forbidden in ["fail", "Panic"] {
        assert!(
            !content.contains(forbidden),
            "burst window contains forbidden string {forbidden:?}\ncaptured content:\n{content}"
        );
    }

    session.expect_output_since(mark, "rose>", BURST_BUDGET);
}
