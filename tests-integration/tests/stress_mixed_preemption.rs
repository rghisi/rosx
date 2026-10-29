use std::time::Duration;
use tests_integration::qemu::QemuSession;

const BOOT_BUDGET: Duration = Duration::from_secs(90);
const MEM_STARTED_BUDGET: Duration = Duration::from_secs(60);
const FIRST_SLICE_BUDGET: Duration = Duration::from_secs(120);
const DIAGNOSTIC_TAIL_CHARS: usize = 2000;

const MIX_STARTED: &str = "[MixWorker] Starting Mixed Load Test...";
const MEM_STARTED: &str = "[MemWorker] Starting Allocation/Deallocation Stress Test...";
const CTX_STARTED: &str = "[CtxWorker] Starting Contention Runner...";
const MEM_PROGRESS_50: &str = "Progress: 50/1000";

#[test]
fn busy_mem_worker_is_preempted_for_ctx_worker() {
    let mut session = QemuSession::spawn();
    session.expect_output("rose>", BOOT_BUDGET);
    let mark = session.mark();
    session.send_line("stress mixed");
    session.expect_output_since(mark, MIX_STARTED, MEM_STARTED_BUDGET);
    session.expect_output_since(mark, MEM_STARTED, MEM_STARTED_BUDGET);

    let either_marker = |delta: &str| delta.contains(CTX_STARTED) || delta.contains(MEM_PROGRESS_50);
    if session
        .wait_until_since(mark, either_marker, FIRST_SLICE_BUDGET)
        .is_err()
    {
        panic!(
            "neither {CTX_STARTED:?} nor {MEM_PROGRESS_50:?} appeared within {FIRST_SLICE_BUDGET:?}\ncaptured output tail:\n{}",
            tail_of(&session.content_since(mark)),
        );
    }

    let content = session.content_since(mark);
    match (content.find(CTX_STARTED), content.find(MEM_PROGRESS_50)) {
        (Some(ctx), Some(progress)) => assert!(
            ctx < progress,
            "[CtxWorker] only started after MemWorker's first progress line: timer preemption is not reaching the running task\ncaptured output tail:\n{}",
            tail_of(&content),
        ),
        (Some(_), None) => {}
        (None, Some(_)) => panic!(
            "[CtxWorker] never got a slice before MemWorker's first progress line: timer preemption is not reaching the running task\ncaptured output tail:\n{}",
            tail_of(&content),
        ),
        (None, None) => unreachable!("wait_until_since fired without either marker"),
    }
}

fn tail_of(content: &str) -> String {
    let char_count = content.chars().count();
    if char_count <= DIAGNOSTIC_TAIL_CHARS {
        return content.to_string();
    }
    content.chars().skip(char_count - DIAGNOSTIC_TAIL_CHARS).collect()
}
