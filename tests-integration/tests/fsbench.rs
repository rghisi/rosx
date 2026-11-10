use std::time::Duration;
use tests_integration::qemu::QemuSession;

const BOOT_BUDGET: Duration = Duration::from_secs(120);
const BENCH_BUDGET: Duration = Duration::from_secs(150);

#[test]
fn fsbench_control_runs_and_reports_a_best_line() {
    let mut session = QemuSession::spawn();
    session.expect_output("rose>", BOOT_BUDGET);
    let mark = session.mark();
    session.send_line("fsbench");
    session.expect_output_since(mark, "bench.done suite=fsbench", BENCH_BUDGET);
    let output = session.content_since(mark);
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
    for phase in ["read_full", "read_small", "elf_load"] {
        assert!(
            output.contains(&format!("bench.fsbench_{phase}.best")),
            "{phase} best line missing:\n{output}"
        );
    }
    println!("{output}");
}
