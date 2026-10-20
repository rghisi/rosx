#![no_std]

extern crate alloc;

use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use usrlib::syscall::Syscall;
use usrlib::{print, println};

static SHARED_COUNTER: AtomicUsize = AtomicUsize::new(0);
static S2_STOP: AtomicBool = AtomicBool::new(false);
static S2_SPIN_TICKS: AtomicUsize = AtomicUsize::new(0);

const INCREMENTS_PER_WORKER: usize = 1000;
const YIELD_EVERY: usize = 100;
const PROGRESS_EVERY: usize = 250;
const S2_PROGRESS_EVERY: usize = 5_000_000;

fn main() {
    scenario_shared_counter_spawn_and_join();
    scenario_spin_wait_cost();
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

fn scenario_spin_wait_cost() {
    S2_STOP.store(false, Ordering::SeqCst);
    S2_SPIN_TICKS.store(0, Ordering::SeqCst);
    println!("S2 SPAWN pure spin worker waiting on an atomic flag");
    let spinner = Syscall::exec(worker_spin_on_flag as usize);
    Syscall::sleep(500);
    Syscall::sleep(1);
    Syscall::sleep(1);
    Syscall::sleep(1);
    Syscall::sleep(1);
    S2_STOP.store(true, Ordering::SeqCst);
    if let Some(handle) = spinner {
        Syscall::wait_future(handle);
    }
    println!("S2 SPIN_TICKS={}", S2_SPIN_TICKS.load(Ordering::SeqCst));
}

fn worker_spin_on_flag() {
    let mut spins: usize = 0;
    while !S2_STOP.load(Ordering::SeqCst) {
        spins += 1;
        if spins % S2_PROGRESS_EVERY == 0 {
            println!("S2 spin ticks: {}", spins);
        }
    }
    S2_SPIN_TICKS.store(spins, Ordering::SeqCst);
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
