use std::time::Duration;
use tests_integration::qemu::QemuSession;

const BOOT_BUDGET: Duration = Duration::from_secs(120);
const BENCH_BUDGET: Duration = Duration::from_secs(150);

#[test]
#[ignore = "benchmark workload, not a per-run correctness gate; run with -- --ignored"]
fn fsbench_control_runs_and_reports_a_best_line() {
    let mut session = QemuSession::spawn();
    session.expect_output("rose>", BOOT_BUDGET);
    let mark = session.mark();
    session.send_line("fsbench /fsbrun 1");
    session.expect_output_since(mark, "bench.done suite=fsbench", BENCH_BUDGET);
    let output = session.content_since(mark);
    assert!(
        output.contains("bench.start suite=fsbench dir=/fsbrun phases=1"),
        "gated run banner missing:\n{output}"
    );
    assert!(
        output.contains("fsbench mkdir /fsbrun"),
        "run-directory preparation missing:\n{output}"
    );
    assert!(
        output.contains("bench.fsbench_control.best"),
        "control best line missing:\n{output}"
    );
    assert!(
        output.contains("bench.control.sink"),
        "control digest sink missing:\n{output}"
    );
    println!("{output}");
}
