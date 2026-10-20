#![no_std]

extern crate alloc;

use core::sync::atomic::{AtomicUsize, Ordering};
use usrlib::syscall::Syscall;
use usrlib::{print, println};

static SHARED_COUNTER: AtomicUsize = AtomicUsize::new(0);

const INCREMENTS_PER_WORKER: usize = 1000;
const YIELD_EVERY: usize = 100;
const PROGRESS_EVERY: usize = 250;

fn main() {
    scenario_shared_counter_spawn_and_join();
}

fn scenario_shared_counter_spawn_and_join() {
    SHARED_COUNTER.store(0, Ordering::SeqCst);
    println!("S1 SPAWN two workers over one atomic counter");
    let first_worker = Syscall::exec(worker_a as usize);
    let second_worker = Syscall::exec(worker_b as usize);
    if let Some(handle) = first_worker {
        Syscall::wait_future(handle);
    }
    if let Some(handle) = second_worker {
        Syscall::wait_future(handle);
    }
    println!("S1 FINAL={}", SHARED_COUNTER.load(Ordering::SeqCst));
}

fn worker_a() {
    increment_shared_counter('A');
}

fn worker_b() {
    increment_shared_counter('B');
}

fn increment_shared_counter(worker: char) {
    for step in 1..=INCREMENTS_PER_WORKER {
        SHARED_COUNTER.fetch_add(1, Ordering::SeqCst);
        if step % PROGRESS_EVERY == 0 {
            println!(
                "S1 WORKER {} AT {} COUNTER {}",
                worker,
                step,
                SHARED_COUNTER.load(Ordering::SeqCst)
            );
        }
        if step % YIELD_EVERY == 0 {
            Syscall::task_yield();
        }
    }
}
