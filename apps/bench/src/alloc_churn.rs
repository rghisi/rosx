use crate::windows;
use alloc::alloc::{alloc, dealloc};
use alloc::format;
use alloc::vec::Vec;
use core::alloc::Layout;
use usrlib::println;

const ALIGNMENT: usize = 8;
const BAND_LOW: usize = 256;
const BAND_HIGH: usize = 1024;
const SIZE_CLASSES: u64 = 7;

pub fn run() {
    let detail = format!(
        "shape=mixed align={} band={}-{}",
        ALIGNMENT, BAND_LOW, BAND_HIGH
    );
    println!("bench.start suite=alloc_churn {}", detail);
    let mut held: Vec<(*mut u8, usize)> = Vec::with_capacity(BAND_HIGH);
    let mut state = 0x9E3779B97F4A7C15u64;
    windows::run_phase("alloc_churn", &detail, &mut || {
        churn_once(&mut held, &mut state)
    });
    free_all(&mut held);
    println!("bench.done suite=alloc_churn");
}

fn next_random(state: &mut u64) -> u64 {
    let mut x = *state;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    *state = x;
    x
}

fn churn_once(held: &mut Vec<(*mut u8, usize)>, state: &mut u64) -> bool {
    let roll = next_random(state);
    let release_first = held.len() >= BAND_HIGH || (held.len() > BAND_LOW && roll & 1 == 1);
    if release_first && !held.is_empty() {
        let index = (roll >> 16) as usize % held.len();
        let (block, size) = held.swap_remove(index);
        release_block(block, size);
        true
    } else {
        allocate_block(held, roll)
    }
}

fn allocate_block(held: &mut Vec<(*mut u8, usize)>, roll: u64) -> bool {
    let size = 16usize << (roll % SIZE_CLASSES);
    let Ok(layout) = Layout::from_size_align(size, ALIGNMENT) else {
        return false;
    };
    let block = unsafe { alloc(layout) };
    if block.is_null() {
        return false;
    }
    held.push((block, size));
    true
}

fn release_block(block: *mut u8, size: usize) {
    let Ok(layout) = Layout::from_size_align(size, ALIGNMENT) else {
        return;
    };
    unsafe { dealloc(block, layout) };
}

fn free_all(held: &mut Vec<(*mut u8, usize)>) {
    while let Some((block, size)) = held.pop() {
        release_block(block, size);
    }
}
