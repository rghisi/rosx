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
