use std::time::Duration;
use tests_integration::qemu::QemuSession;

const BOOT_BUDGET: Duration = Duration::from_secs(90);
const START_BUDGET: Duration = Duration::from_secs(60);
const FIRST_CACHED_PHASE_BUDGET: Duration = Duration::from_secs(120);
const CACHED_SWEEP_BUDGET: Duration = Duration::from_secs(600);
const CHURN_START_BUDGET: Duration = Duration::from_secs(60);
const CHURN_BEST_BUDGET: Duration = Duration::from_secs(180);
const CHURN_DONE_BUDGET: Duration = Duration::from_secs(60);

#[test]
#[ignore = "alloc_cached/alloc_churn are the oracle for enabling the slab allocator; keep ignored until the kernel gains task-group ownership (docs/todo.md), which makes per-image caching sound for the exec-spawning bench app"]
fn bench_allocator_suites_report_machine_readable_lines() {
    let mut session = QemuSession::spawn();
    let boot_mark = session.mark();
    session.expect_output("rose>", BOOT_BUDGET);

    let before_list = session.mark();
    session.send_line("bench list");
    session.expect_output_since(
        before_list,
        "alloc_cached, alloc_churn",
        Duration::from_secs(15),
    );

    let cached_mark = session.mark();
    session.send_line("bench alloc_cached");
    session.expect_output_since(cached_mark, "bench.start suite=alloc_cached", START_BUDGET);
    session.expect_output_since(
        cached_mark,
        "bench.alloc_cached.best size=64 ",
        FIRST_CACHED_PHASE_BUDGET,
    );
    session.expect_output_since(
        cached_mark,
        "bench.alloc_cached.best size=4194304 ",
        CACHED_SWEEP_BUDGET,
    );
    session.expect_output_since(
        cached_mark,
        "bench.done suite=alloc_cached",
        CACHED_SWEEP_BUDGET,
    );

    let churn_mark = session.mark();
    session.send_line("bench alloc_churn");
    session.expect_output_since(
        churn_mark,
        "bench.start suite=alloc_churn",
        CHURN_START_BUDGET,
    );
    session.expect_output_since(
        churn_mark,
        "bench.alloc_churn.best shape=mixed ",
        CHURN_BEST_BUDGET,
    );
    session.expect_output_since(
        churn_mark,
        "bench.done suite=alloc_churn",
        CHURN_DONE_BUDGET,
    );

    let transcript = session.content_since(boot_mark);
    assert!(
        !transcript.contains("BAD-FREE"),
        "kernel reported a bad free during the allocator suites:\n{}",
        transcript
    );
}
