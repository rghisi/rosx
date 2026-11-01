use core::hint::black_box;
use usrlib::println;
use usrlib::syscall::Syscall;

const ALIGNMENT: usize = 8;
const MIN_WINDOW_MS: u64 = 200;
const TARGET_WINDOW_MS: u64 = 500;
const MAX_ITERATIONS: u64 = 4_000_000;
const MEASURE_REPS: usize = 3;

const SIZES: &[usize] = &[
    16,
    24,
    32,
    64,
    128,
    256,
    512,
    1000,
    1024,
    2048,
    4096,
    4097,
    8192,
    16384,
    32768,
    65536,
    131072,
    262144,
    524288,
    1048576,
    2097152,
    4194304,
];

pub fn run() {
    println!("bench.start suite=alloc align={} min_window_ms={}", ALIGNMENT, MIN_WINDOW_MS);
    for size in SIZES {
        run_size(*size);
    }
    println!("bench.done suite=alloc");
}

fn run_size(size: usize) {
    let Some(iterations) = calibrate(size) else {
        println!("bench.alloc size={} align={} result=oom phase=calibrate", size, ALIGNMENT);
        return;
    };
    warm_up(size, iterations);
    let mut best_ms: Option<u64> = None;
    for rep in 1..=MEASURE_REPS {
        let Some(elapsed_ms) = time_roundtrip(size, iterations) else {
            println!("bench.alloc size={} align={} result=oom rep={}", size, ALIGNMENT, rep);
            return;
        };
        best_ms = Some(best_ms.map_or(elapsed_ms, |best| best.min(elapsed_ms)));
        println!(
            "bench.alloc size={} align={} rep={} iterations={} elapsed_ms={} allocs_per_sec={} ns_per_alloc={}",
            size,
            ALIGNMENT,
            rep,
            iterations,
            elapsed_ms,
            allocs_per_sec(iterations, elapsed_ms),
            ns_per_alloc(iterations, elapsed_ms)
        );
    }
    if let Some(elapsed_ms) = best_ms {
        println!(
            "bench.alloc.best size={} align={} iterations={} elapsed_ms={} allocs_per_sec={} ns_per_alloc={} reps={}",
            size,
            ALIGNMENT,
            iterations,
            elapsed_ms,
            allocs_per_sec(iterations, elapsed_ms),
            ns_per_alloc(iterations, elapsed_ms),
            MEASURE_REPS
        );
    }
}

fn calibrate(size: usize) -> Option<u64> {
    let mut iterations = 64u64;
    loop {
        let elapsed_ms = time_roundtrip(size, iterations)?;
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

fn warm_up(size: usize, iterations: u64) {
    let warmup_iterations = (iterations / 4).max(1);
    let _ = time_roundtrip(size, warmup_iterations);
}

fn time_roundtrip(size: usize, iterations: u64) -> Option<u64> {
    let start = Syscall::now();
    for _ in 0..iterations {
        let block = Syscall::alloc(size, ALIGNMENT);
        if block.is_null() {
            return None;
        }
        black_box(block);
        Syscall::dealloc(block, size, ALIGNMENT);
    }
    Some(Syscall::now() - start)
}

fn allocs_per_sec(iterations: u64, elapsed_ms: u64) -> u64 {
    iterations.saturating_mul(1000) / elapsed_ms.max(1)
}

fn ns_per_alloc(iterations: u64, elapsed_ms: u64) -> u64 {
    elapsed_ms.saturating_mul(1_000_000) / iterations.max(1)
}
