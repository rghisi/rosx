use std::time::Duration;
use tests_integration::qemu::QemuSession;

const BOOT_BUDGET: Duration = Duration::from_secs(90);
const CORRUPT_TIMEOUT: Duration = Duration::from_secs(30);
const PANIC_TIMEOUT: Duration = Duration::from_secs(30);

#[test]
fn boom_guard_corruption_panics_the_kernel_naming_the_task() {
    let mut session = QemuSession::spawn();
    session.expect_output("rose>", BOOT_BUDGET);
    let mark = session.mark();
    session.send_line("boom");
    session.expect_output_since(mark, "BOOM corrupted guard", CORRUPT_TIMEOUT);
    session.expect_output_since(mark, "!!! PANIC !!!", PANIC_TIMEOUT);
    session.expect_output_since(mark, "stack overflow: task", PANIC_TIMEOUT);
    session.expect_output_since(mark, "guard word at", PANIC_TIMEOUT);
}
