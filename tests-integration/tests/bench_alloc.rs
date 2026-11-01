use std::time::Duration;
use tests_integration::qemu::QemuSession;

const BOOT_BUDGET: Duration = Duration::from_secs(90);
const START_BUDGET: Duration = Duration::from_secs(60);
const FIRST_PHASE_BUDGET: Duration = Duration::from_secs(120);
const SWEEP_BUDGET: Duration = Duration::from_secs(420);

#[test]
fn bench_alloc_reports_machine_readable_lines() {
    let mut session = QemuSession::spawn();
    session.expect_output("rose>", BOOT_BUDGET);

    let before_list = session.mark();
    session.send_line("bench list");
    session.expect_output_since(before_list, "Available benchmarks: alloc", Duration::from_secs(15));

    let mark = session.mark();
    session.send_line("bench alloc");
    session.expect_output_since(mark, "bench.start", START_BUDGET);
    session.expect_output_since(mark, "bench.alloc.best size=64 ", FIRST_PHASE_BUDGET);
    session.expect_output_since(mark, "bench.alloc.best size=4194304 ", SWEEP_BUDGET);
    session.expect_output_since(mark, "bench.done", SWEEP_BUDGET);
}
