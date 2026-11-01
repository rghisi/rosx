#![no_std]

extern crate alloc;

use alloc::boxed::Box;
use core::ptr::null_mut;
use core::sync::atomic::{AtomicBool, AtomicPtr, AtomicUsize, Ordering};
use usrlib::handle::UserConnectionHandle;
use usrlib::ipc::ipc_send_value;
use usrlib::syscall::Syscall;
use usrlib::{print, println};

static SHARED_COUNTER: AtomicUsize = AtomicUsize::new(0);
static S2_STOP: AtomicBool = AtomicBool::new(false);
static S2_SPIN_TICKS: AtomicUsize = AtomicUsize::new(0);
static SHARED_CONNECTION: AtomicPtr<UserConnectionHandle> = AtomicPtr::new(null_mut());

const INCREMENTS_PER_WORKER: usize = 1000;
const YIELD_EVERY: usize = 100;
const PROGRESS_EVERY: usize = 250;
const S2_PROGRESS_EVERY: usize = 5_000_000;
const S3_WORKER_SEND_TAG: usize = 698_366;
const S3_MAIN_SEND_TAG: usize = 999_991;
const S4_WORKER_LAPS: usize = 20;
const S4_SLEEP_MS: u64 = 200;

fn main() {
    match usrlib::args().get(1).copied() {
        Some("hazard") => scenario_image_lifetime_hazard(),
        _ => run_safe_scenarios(),
    }
}

fn run_safe_scenarios() {
    scenario_shared_counter_spawn_and_join();
    scenario_spin_wait_cost();
    scenario_shared_connection_ownership();
}

fn scenario_shared_counter_spawn_and_join() {
    SHARED_COUNTER.store(0, Ordering::SeqCst);
    println!("S1 SPAWN two workers over one atomic counter");
    let first_worker = Syscall::spawn(worker_a as usize);
    let second_worker = Syscall::spawn(worker_b as usize);
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
    let spinner = Syscall::spawn(worker_spin_on_flag as usize);
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

fn scenario_shared_connection_ownership() {
    let Ok(connection) = Syscall::ipc_connect("RANDOM") else {
        println!("S3 SKIP no RANDOM server");
        return;
    };
    println!("S3 CONNECTED {}", connection);
    let leaked = Box::leak(Box::new(connection));
    SHARED_CONNECTION.store(leaked as *mut UserConnectionHandle, Ordering::SeqCst);
    let worker = Syscall::spawn(worker_sends_on_shared_connection as usize);
    if let Some(handle) = worker {
        Syscall::wait_future(handle);
    }
    match ipc_send_value(connection, S3_MAIN_SEND_TAG) {
        Ok(()) => println!("S3 MAIN_SEND=ok"),
        Err(e) => println!("S3 MAIN_SEND={}", e),
    }
    match receive_if_pending(connection) {
        Some(S3_WORKER_SEND_TAG) => println!(
            "S3 CLIENT_MAILBOX_FIRST={} worker_message_misrouted_not_rejected",
            S3_WORKER_SEND_TAG
        ),
        Some(value) => println!("S3 CLIENT_MAILBOX_FIRST={}", value),
        None => println!("S3 CLIENT_MAILBOX_EMPTY"),
    }
    Syscall::ipc_disconnect(connection);
}

fn receive_if_pending(connection: UserConnectionHandle) -> Option<usize> {
    let handle = Syscall::ipc_receive_message(connection)?;
    if !Syscall::is_future_completed(handle) {
        return None;
    }
    let message = Syscall::wait_for_message(handle).ok()?;
    let mut bytes = [0u8; core::mem::size_of::<usize>()];
    bytes.copy_from_slice(&message.data()[..core::mem::size_of::<usize>()]);
    Some(usize::from_ne_bytes(bytes))
}

fn worker_sends_on_shared_connection() {
    let shared = SHARED_CONNECTION.load(Ordering::SeqCst);
    if shared.is_null() {
        println!("S3 WORKER_NO_CONNECTION");
        return;
    }
    let connection = unsafe { *shared };
    match ipc_send_value(connection, S3_WORKER_SEND_TAG) {
        Ok(()) => println!("S3 WORKER_SEND=ok"),
        Err(e) => println!("S3 WORKER_SEND={}", e),
    }
}

fn scenario_image_lifetime_hazard() {
    println!("S4 SPAWN worker that outlives main without join");
    Syscall::spawn(worker_outliving_main as usize);
    println!("S4 MAIN_EXITING");
}

fn worker_outliving_main() {
    for lap in 1..=S4_WORKER_LAPS {
        println!("S4 WORKER_ALIVE {}", lap);
        Syscall::sleep(S4_SLEEP_MS);
    }
    println!("S4 WORKER_DONE");
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
