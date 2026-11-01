use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::hint::black_box;
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use usrlib::handle::UserFutureHandle;
use usrlib::println;
use usrlib::syscall::Syscall;

const SPIN_WORKERS: usize = 4;
const SPIN_BUDGET: usize = 3_000_000;
const SPIN_CHECKPOINT: usize = 16_384;

const CONTENTION_SPIN_WORKERS: usize = 2;
const YIELD_WORKERS: usize = 1;
const YIELD_BUDGET: usize = 1_000_000;
const YIELD_CHUNK: usize = 500;

const SAMPLE_MS: u64 = 100;
const EQUAL_FAMILY_STALL_LIMIT: usize = 10;
const MIXED_FAMILY_STALL_LIMIT: usize = 30;

static SPIN_PROGRESS: [AtomicUsize; SPIN_WORKERS] = [const { AtomicUsize::new(0) }; SPIN_WORKERS];
static SPIN_DONE: [AtomicBool; SPIN_WORKERS] = [const { AtomicBool::new(false) }; SPIN_WORKERS];
static NEXT_SPIN_ID: AtomicUsize = AtomicUsize::new(0);

static YIELD_PROGRESS: [AtomicUsize; YIELD_WORKERS] = [const { AtomicUsize::new(0) }; YIELD_WORKERS];
static YIELD_DONE: [AtomicBool; YIELD_WORKERS] = [const { AtomicBool::new(false) }; YIELD_WORKERS];
static NEXT_YIELD_ID: AtomicUsize = AtomicUsize::new(0);

struct WorkerSet {
    label: &'static str,
    workers: usize,
    progress: &'static [AtomicUsize],
    done: &'static [AtomicBool],
    budget: usize,
    stall_limit: usize,
}

fn spin_set(stall_limit: usize) -> WorkerSet {
    WorkerSet {
        label: "spin",
        workers: SPIN_WORKERS,
        progress: &SPIN_PROGRESS,
        done: &SPIN_DONE,
        budget: SPIN_BUDGET,
        stall_limit,
    }
}

fn contention_spin_set() -> WorkerSet {
    WorkerSet {
        workers: CONTENTION_SPIN_WORKERS,
        ..spin_set(MIXED_FAMILY_STALL_LIMIT)
    }
}

fn yield_set() -> WorkerSet {
    WorkerSet {
        label: "yield",
        workers: YIELD_WORKERS,
        progress: &YIELD_PROGRESS,
        done: &YIELD_DONE,
        budget: YIELD_BUDGET,
        stall_limit: MIXED_FAMILY_STALL_LIMIT,
    }
}

pub fn run() {
    println!("[CtxWorker] Starting Context Switch Test...");

    let spinners = spin_set(EQUAL_FAMILY_STALL_LIMIT);
    reset(&[&spinners]);
    match spawn_all("spin", worker_spin, SPIN_WORKERS) {
        Ok(handles) => {
            if let Err(reason) = monitor(&[&spinners]) {
                println!("[CtxWorker] [FAIL] {reason}");
                return;
            }
            join_all(handles);
            if let Err(problem) = report(&spinners) {
                println!("[CtxWorker] [FAIL] {problem}");
                return;
            }
        }
        Err(reason) => {
            println!("[CtxWorker] [FAIL] {reason}");
            return;
        }
    }

    let spinners = contention_spin_set();
    let yielders = yield_set();
    reset(&[&spinners, &yielders]);
    let mut handles = match spawn_all("spin", worker_spin, spinners.workers) {
        Ok(handles) => handles,
        Err(reason) => {
            println!("[CtxWorker] [FAIL] {reason}");
            return;
        }
    };
    match spawn_all("yield", worker_yield_spin, YIELD_WORKERS) {
        Ok(more) => handles.extend(more),
        Err(reason) => {
            println!("[CtxWorker] [FAIL] {reason}");
            return;
        }
    }
    if let Err(reason) = monitor(&[&spinners, &yielders]) {
        println!("[CtxWorker] [FAIL] {reason}");
        return;
    }
    join_all(handles);
    if let Err(problem) = report(&spinners).and(report(&yielders)) {
        println!("[CtxWorker] [FAIL] {problem}");
        return;
    }

    println!(
        "\n[CtxWorker] [PASS] Context Switch Test Completed ({} spinners x {} rounds under preemption, contention round with {} spinner + 1 yielder x {} yields)",
        SPIN_WORKERS,
        SPIN_BUDGET,
        CONTENTION_SPIN_WORKERS,
        YIELD_BUDGET / YIELD_CHUNK
    );
}

pub fn worker_context_switch() {
    println!("[CtxWorker] Starting Contention Runner...");

    let spinners = contention_spin_set();
    let yielders = yield_set();
    reset(&[&spinners, &yielders]);
    let mut handles = match spawn_all("spin", worker_spin, spinners.workers) {
        Ok(handles) => handles,
        Err(reason) => {
            println!("[CtxWorker] [FAIL] {reason}");
            return;
        }
    };
    match spawn_all("yield", worker_yield_spin, YIELD_WORKERS) {
        Ok(more) => handles.extend(more),
        Err(reason) => {
            println!("[CtxWorker] [FAIL] {reason}");
            return;
        }
    }
    if let Err(reason) = monitor(&[&spinners, &yielders]) {
        println!("[CtxWorker] [FAIL] {reason}");
        return;
    }
    join_all(handles);
    if let Err(problem) = report(&spinners).and(report(&yielders)) {
        println!("[CtxWorker] [FAIL] {problem}");
        return;
    }

    println!("\n[CtxWorker] [PASS] Contention Runner Completed");
}

fn worker_spin() {
    let id = NEXT_SPIN_ID.fetch_add(1, Ordering::SeqCst);
    if id >= SPIN_WORKERS {
        return;
    }
    let mut mix = 0x9E37_79B9_7F4A_7C15u64 ^ (id as u64);
    for round in 1..=SPIN_BUDGET {
        mix = mix.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        if round % SPIN_CHECKPOINT == 0 {
            SPIN_PROGRESS[id].store(round, Ordering::SeqCst);
        }
    }
    black_box(mix);
    SPIN_PROGRESS[id].store(SPIN_BUDGET, Ordering::SeqCst);
    SPIN_DONE[id].store(true, Ordering::SeqCst);
}

fn worker_yield_spin() {
    let id = NEXT_YIELD_ID.fetch_add(1, Ordering::SeqCst);
    if id >= YIELD_WORKERS {
        return;
    }
    let mut mix = 0xDEAD_BEEF_CAFE_B0B0u64 ^ (id as u64);
    for round in 1..=YIELD_BUDGET {
        mix = mix.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        if round % YIELD_CHUNK == 0 {
            Syscall::task_yield();
        }
    }
    black_box(mix);
    YIELD_PROGRESS[id].store(YIELD_BUDGET, Ordering::SeqCst);
    YIELD_DONE[id].store(true, Ordering::SeqCst);
}

fn spawn_all(label: &str, entry: fn(), count: usize) -> Result<Vec<UserFutureHandle>, String> {
    let mut handles = Vec::new();
    for _ in 0..count {
        match Syscall::spawn(entry as *const () as usize) {
            Some(handle) => handles.push(handle),
            None => return Err(format!("cannot spawn {label} worker")),
        }
    }
    Ok(handles)
}

fn join_all(handles: Vec<UserFutureHandle>) {
    for handle in handles {
        Syscall::wait_future(handle);
    }
}

fn monitor(sets: &[&WorkerSet]) -> Result<(), String> {
    let total = sets.iter().map(|set| set.workers).sum();
    let mut last = vec![0usize; total];
    let mut stalled = vec![0usize; total];

    loop {
        Syscall::sleep(SAMPLE_MS);
        let mut all_done = true;
        let mut base = 0usize;
        for set in sets {
            for offset in 0..set.workers {
                let slot = base + offset;
                if set.done[offset].load(Ordering::SeqCst) {
                    stalled[slot] = 0;
                    continue;
                }
                all_done = false;
                let value = set.progress[offset].load(Ordering::SeqCst);
                if value == last[slot] {
                    stalled[slot] += 1;
                } else {
                    stalled[slot] = 0;
                }
                if stalled[slot] > set.stall_limit {
                    return Err(format!(
                        "{} worker {} starved: no progress across {} samples at {}/{}",
                        set.label, offset, stalled[slot], value, set.budget
                    ));
                }
                last[slot] = value;
            }
            base += set.workers;
        }
        if all_done {
            return Ok(());
        }
    }
}

fn report(set: &WorkerSet) -> Result<(), String> {
    let mut problems = Vec::new();
    for offset in 0..set.workers {
        let value = set.progress[offset].load(Ordering::SeqCst);
        println!(
            "[CtxWorker] {} worker {}: {}/{}",
            set.label, offset, value, set.budget
        );
        if value != set.budget || !set.done[offset].load(Ordering::SeqCst) {
            problems.push(format!(
                "{} worker {} stopped at {}/{}",
                set.label, offset, value, set.budget
            ));
        }
    }
    if problems.is_empty() {
        Ok(())
    } else {
        Err(problems.join("; "))
    }
}

fn reset(sets: &[&WorkerSet]) {
    NEXT_SPIN_ID.store(0, Ordering::SeqCst);
    NEXT_YIELD_ID.store(0, Ordering::SeqCst);
    for set in sets {
        for slot in set.progress {
            slot.store(0, Ordering::SeqCst);
        }
        for flag in set.done {
            flag.store(false, Ordering::SeqCst);
        }
    }
}
