use crate::seed::lcg_fill;
use crate::windows;
use usrlib::println;

const CONTROL_BYTES: usize = 64 * 1024;

pub fn run() {
    println!("bench.start suite=fsbench.phase=control shape=cpu");
    let buffer = lcg_fill(CONTROL_BYTES);
    let mut rounds: u64 = 0;
    let mut digest: u64 = 0;
    windows::run_phase(
        "fsbench_control",
        "shape=cpu bytes=65536",
        &mut || {
            digest = fletcher32(&buffer);
            rounds = rounds.wrapping_add(1);
            true
        },
    );
    println!("bench.control.sink rounds={rounds} digest={digest}");
}

pub(crate) fn fletcher32(data: &[u8]) -> u64 {
    let mut sum1: u32 = 0xFFFF;
    let mut sum2: u32 = 0xFFFF;
    for chunk in data.chunks_exact(2) {
        let word = u32::from(chunk[0]) | (u32::from(chunk[1]) << 8);
        sum1 = sum1.wrapping_add(word);
        sum2 = sum2.wrapping_add(sum1);
        if sum1 >= 0xFFFF {
            sum1 -= 0xFFFF;
        }
        if sum2 >= 0xFFFF {
            sum2 -= 0xFFFF;
        }
    }
    ((sum2 as u64) << 32) | sum1 as u64
}
