use usrlib::println;
use usrlib::syscall::Syscall;

const MIN_WINDOW_MS: u64 = 200;
const TARGET_WINDOW_MS: u64 = 500;
const MAX_ITERATIONS: u64 = 4_000_000;
const MEASURE_REPS: usize = 3;

pub(crate) fn run_phase(phase: &str, detail: &str, operation: &dyn Fn() -> bool) {
    let Some(iterations) = calibrate(operation) else {
        println!("bench.{phase} {detail} result=oom stage=calibrate");
        return;
    };
    warm_up(operation, iterations);
    let mut best_ms: Option<u64> = None;
    for rep in 1..=MEASURE_REPS {
        let Some(elapsed_ms) = time_window(operation, iterations) else {
            println!("bench.{phase} {detail} result=oom rep={rep}");
            return;
        };
        best_ms = Some(best_ms.map_or(elapsed_ms, |best| best.min(elapsed_ms)));
        let ops_per_sec = ops_per_sec(iterations, elapsed_ms);
        let ns = ns_per_op(iterations, elapsed_ms);
        println!(
            "bench.{phase} {detail} rep={rep} iterations={iterations} elapsed_ms={elapsed_ms} ops_per_sec={ops_per_sec} ns_per_op={ns}"
        );
    }
    if let Some(elapsed_ms) = best_ms {
        let ops_per_sec = ops_per_sec(iterations, elapsed_ms);
        let ns = ns_per_op(iterations, elapsed_ms);
        println!(
            "bench.{phase}.best {detail} iterations={iterations} elapsed_ms={elapsed_ms} ops_per_sec={ops_per_sec} ns_per_op={ns} reps={MEASURE_REPS}"
        );
    }
}

fn calibrate(operation: &dyn Fn() -> bool) -> Option<u64> {
    let mut iterations = 64u64;
    loop {
        let elapsed_ms = time_window(operation, iterations)?;
        if elapsed_ms >= MIN_WINDOW_MS || iterations >= MAX_ITERATIONS {
            return Some(iterations);
        }
        let projected = if elapsed_ms == 0 {
            iterations.saturating_mul(16)
        } else {
            iterations.saturating_mul(TARGET_WINDOW_MS / elapsed_ms.max(1))
        };
        iterations = projected.max(iterations.saturating_mul(2)).min(MAX_ITERATIONS);
    }
}

fn warm_up(operation: &dyn Fn() -> bool, iterations: u64) {
    let warmup_iterations = (iterations / 4).max(1);
    let _ = time_window(operation, warmup_iterations);
}

fn time_window(operation: &dyn Fn() -> bool, iterations: u64) -> Option<u64> {
    let start = Syscall::now();
    for _ in 0..iterations {
        if !operation() {
            return None;
        }
    }
    Some(Syscall::now() - start)
}

fn ops_per_sec(iterations: u64, elapsed_ms: u64) -> u64 {
    iterations.saturating_mul(1000) / elapsed_ms.max(1)
}

fn ns_per_op(iterations: u64, elapsed_ms: u64) -> u64 {
    elapsed_ms.saturating_mul(1_000_000) / iterations.max(1)
}
