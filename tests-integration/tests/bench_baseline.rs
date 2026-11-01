use std::fs;
use std::time::Duration;
use tests_integration::bench_history::{self, Context};
use tests_integration::qemu::QemuSession;

const BOOT_BUDGET: Duration = Duration::from_secs(90);
const SWEEP_BUDGET: Duration = Duration::from_secs(600);
const DEFAULT_TOLERANCE_PCT: u64 = 20;

#[test]
#[ignore = "performance history gate; run with -- --ignored, set BENCH_RECORD=1 to append a new record"]
fn bench_history_gate() {
    let mut session = QemuSession::spawn();
    session.expect_output("rose>", BOOT_BUDGET);

    let mark = session.mark();
    session.send_line("bench");
    session.expect_output_since(mark, "bench.start suite=alloc", BOOT_BUDGET);
    session.expect_output_since(mark, "=== Bench Finished ===", SWEEP_BUDGET);

    let content = session.content_since(mark);
    let measurements = bench_history::parse_bench_lines(&content);
    assert!(
        !measurements.is_empty(),
        "no bench best-lines captured from the run\ncaptured tail:\n{}",
        content.chars().rev().take(2000).collect::<String>().chars().rev().collect::<String>()
    );

    if record_requested() {
        let path = bench_history::append_record(
            &bench_history::history_directory(),
            &Context::detect_now(),
            &measurements,
            pull_request_tag().as_deref(),
        )
        .expect("failed to append bench record");
        println!("recorded {} measurements to {}", measurements.len(), path.display());
        return;
    }

    let directory = bench_history::history_directory();
    let latest = bench_history::latest_record(&directory).unwrap_or_else(|| {
        panic!(
            "no bench history found in {}; record one with BENCH_RECORD=1 cargo test -p tests-integration --test bench_baseline -- --ignored",
            directory.display()
        )
    });
    let (header, baseline) = bench_history::parse_record(
        &fs::read_to_string(&latest).expect("failed to read latest bench record"),
    );

    let mismatches = bench_history::context_mismatches(&Context::detect_now(), &header);
    assert!(
        mismatches.is_empty(),
        "cannot compare against {} with a different environment:\n{}\nrecord a new baseline on this host",
        latest.display(),
        mismatches.join("\n")
    );

    let problems = bench_history::compare(&baseline, &measurements, tolerance_pct());
    assert!(
        problems.is_empty(),
        "bench regression against {}:\n{}",
        latest.display(),
        problems.join("\n")
    );
    println!(
        "bench gate passed: {} measurements within {}% of {}",
        measurements.len(),
        tolerance_pct(),
        latest.display()
    );
}

fn record_requested() -> bool {
    matches!(std::env::var("BENCH_RECORD").as_deref(), Ok("1") | Ok("true"))
}

fn pull_request_tag() -> Option<String> {
    std::env::var("BENCH_PR")
        .ok()
        .filter(|value| !value.is_empty())
}

fn tolerance_pct() -> u64 {
    std::env::var("BENCH_TOLERANCE_PCT")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(DEFAULT_TOLERANCE_PCT)
}
