use std::time::Duration;
use tests_integration::qemu::QemuSession;

const BOOT_BUDGET: Duration = Duration::from_secs(90);
const SNAPSHOT_TIMEOUT: Duration = Duration::from_secs(30);
const REPORT_LINES: [&str; 6] = [
    "KSTATSD memory snapshot",
    "used:",
    "free:",
    "total:",
    "largest free block:",
    "free blocks:",
];

#[test]
fn mem_reports_a_memory_snapshot_over_ipc() {
    let mut session = QemuSession::spawn();
    session.expect_output("rose>", BOOT_BUDGET);

    let mark = session.mark();
    session.send_line("mem");
    for needle in REPORT_LINES {
        session.expect_output_since(mark, needle, SNAPSHOT_TIMEOUT);
    }

    let after_report = session.mark();
    session.expect_output_since(after_report, "rose>", SNAPSHOT_TIMEOUT);
}

fn used_kib(session: &mut QemuSession) -> u64 {
    let mark = session.mark();
    session.send_line("mem");
    session.expect_output_since(mark, "uptime:", SNAPSHOT_TIMEOUT);
    let report = session.content_since(mark);
    let line = report
        .lines()
        .find(|line| line.trim_start().starts_with("used:"))
        .expect("the snapshot carries a used line");
    line.split_whitespace()
        .nth(1)
        .expect("the used line carries a KiB count")
        .parse()
        .expect("the used count is a number")
}

#[test]
fn repeated_mem_runs_report_stable_used_bytes() {
    let mut session = QemuSession::spawn();
    session.expect_output("rose>", BOOT_BUDGET);

    let samples: Vec<u64> = (0..5).map(|_| used_kib(&mut session)).collect();
    println!("used KiB across five runs: {samples:?}");

    let spread = samples.iter().max().unwrap() - samples.iter().min().unwrap();
    assert!(spread < 6, "reported memory does not erode across runs: {samples:?}");
}
