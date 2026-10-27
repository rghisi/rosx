use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use usrlib::println;
use crate::random::SimpleRng;

const GUARD_BYTES: usize = 64;
const GUARD_FILL: u8 = 0x5A;
const MAX_CONCURRENT_BLOCKS: usize = 5;
const TOTAL_OPERATIONS: usize = 1000;
const SWEEP_INTERVAL: usize = 50;

const MINIMUM_SIZE: u32 = 16;
const MAXIMUM_SIZE: u32 = 16 * 1024 * 1024;

pub(crate) fn pattern_byte(key: u64, index: usize) -> u8 {
    let mixed = (key ^ (index as u64)).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    (mixed >> 56) as u8
}

struct GuardedBlock {
    id: usize,
    key: u64,
    payload_len: usize,
    region: Vec<u8>,
}

impl GuardedBlock {
    fn new(id: usize, payload_len: usize, key: u64) -> Self {
        let mut region = Vec::with_capacity(payload_len + 2 * GUARD_BYTES);
        for _ in 0..GUARD_BYTES {
            region.push(GUARD_FILL);
        }
        for index in 0..payload_len {
            region.push(pattern_byte(key, index));
        }
        for _ in 0..GUARD_BYTES {
            region.push(GUARD_FILL);
        }
        GuardedBlock {
            id,
            key,
            payload_len,
            region,
        }
    }

    fn verify(&self) -> Option<String> {
        if let Some(offset) = self.guard_failure(&self.region[..GUARD_BYTES]) {
            return Some(format!("underflow guard corrupted at offset -{}", GUARD_BYTES - offset));
        }
        let payload = &self.region[GUARD_BYTES..GUARD_BYTES + self.payload_len];
        for (index, &byte) in payload.iter().enumerate() {
            let expected = pattern_byte(self.key, index);
            if byte != expected {
                return Some(format!(
                    "payload mismatch at index {} (expected 0x{:02x}, found 0x{:02x})",
                    index, expected, byte
                ));
            }
        }
        if let Some(offset) = self.guard_failure(&self.region[GUARD_BYTES + self.payload_len..]) {
            return Some(format!("overflow guard corrupted at offset +{}", offset));
        }
        None
    }

    fn guard_failure(&self, guard: &[u8]) -> Option<usize> {
        guard.iter().position(|&byte| byte != GUARD_FILL)
    }
}

fn sweep(allocations: &[GuardedBlock]) -> Option<(usize, String)> {
    for block in allocations {
        if let Some(reason) = block.verify() {
            return Some((block.id, reason));
        }
    }
    None
}

pub fn run() {
    println!("[MemWorker] Starting Allocation/Deallocation Stress Test...");

    let mut allocations: Vec<GuardedBlock> = Vec::new();
    let mut rng = SimpleRng::new(0x1337);
    let mut total_allocs_performed = 0;
    let mut live_payload_bytes: usize = 0;

    for operation in 1..=TOTAL_OPERATIONS {
        let should_allocate = if allocations.len() >= MAX_CONCURRENT_BLOCKS {
            false
        } else if allocations.is_empty() {
            true
        } else {
            rng.coin()
        };

        if should_allocate {
            let id = total_allocs_performed;
            total_allocs_performed += 1;
            let size = rng.next_range(MINIMUM_SIZE, MAXIMUM_SIZE) as usize;
            let key = rng.next_u64();
            let block = GuardedBlock::new(id, size, key);

            if let Some(reason) = block.verify() {
                println!(
                    "[MemWorker] [FAIL] immediate verification of block {} ({} bytes): {}",
                    id, size, reason
                );
                return;
            }

            live_payload_bytes += size;
            allocations.push(block);
        } else {
            let index = rng.next_range(0, (allocations.len() - 1) as u32) as usize;
            let block = allocations.swap_remove(index);

            if let Some(reason) = block.verify() {
                println!(
                    "[MemWorker] [FAIL] integrity of block {} ({} bytes) on release: {}",
                    block.id, block.payload_len, reason
                );
                return;
            }

            live_payload_bytes -= block.payload_len;
        }

        if operation % SWEEP_INTERVAL == 0 {
            if let Some((id, reason)) = sweep(&allocations) {
                println!(
                    "[MemWorker] [FAIL] sweep at operation {}: block {}: {}",
                    operation, id, reason
                );
                return;
            }
            println!(
                "[MemWorker] Progress: {}/{} - {} live blocks holding {} MB",
                operation,
                TOTAL_OPERATIONS,
                allocations.len(),
                live_payload_bytes / 1024 / 1024
            );
        }
    }

    println!(
        "[MemWorker] Finalizing: verifying remaining {} allocations...",
        allocations.len()
    );
    if let Some((id, reason)) = sweep(&allocations) {
        println!("[MemWorker] [FAIL] final verification: block {}: {}", id, reason);
        return;
    }

    println!(
        "[MemWorker] [PASS] Allocation/Deallocation Stress Test Completed Successfully ({})",
        total_allocs_performed
    );
}
