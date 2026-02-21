use alloc::vec::Vec;
use alloc::boxed::Box;
use usrlib::println;
use crate::random::SimpleRng;

pub struct DataBlock {
    pub magic: usize,
    pub id: usize,
    pub data: Vec<u8>,
}

impl DataBlock {
    pub fn new(id: usize, size: usize, magic: usize) -> Self {
        let mut data = Vec::with_capacity(size);
        for i in 0..size {
            data.push((id as u8).wrapping_add(i as u8));
        }
        DataBlock {
            magic,
            id,
            data,
        }
    }

    pub fn verify(&self, expected_magic: usize) -> bool {
        if self.magic != expected_magic {
            println!("Memory Corruption: Magic number mismatch for block {}! Expected: {:x}, Found: {:x}", self.id, expected_magic, self.magic);
            return false;
        }
        for (i, &val) in self.data.iter().enumerate() {
            if val != (self.id as u8).wrapping_add(i as u8) {
                println!("Memory Corruption: Data mismatch for block {} at index {}! Expected: {}, Found: {}", self.id, i, (self.id as u8).wrapping_add(i as u8), val);
                return false;
            }
        }
        true
    }
}

pub fn run() {
    println!("[MemWorker] Starting Allocation/Deallocation Stress Test...");

    let mut allocations: Vec<(Box<DataBlock>, usize)> = Vec::new();
    let mut rng = SimpleRng::new(0x1337);
    let mut total_allocs_performed = 0;

    const MAX_CONCURRENT_ALLOCS: usize = 5;
    const TOTAL_OPERATIONS: usize = 1000;

    const MINIMUM_SIZE: u32 = 16; //16B
    const MAXIMUM_SIZE: u32 = 16 * 1024 * 1024; //16MB

    let mut allocated: usize = 0;

    for i in 0..TOTAL_OPERATIONS {
        let should_allocate = if allocations.len() >= MAX_CONCURRENT_ALLOCS {
            false
        } else if allocations.is_empty() {
            true
        } else {
            rng.next().is_multiple_of(2)
        };

        if should_allocate {
            let id = total_allocs_performed;
            total_allocs_performed += 1;
            let size = rng.next_range(MINIMUM_SIZE, MAXIMUM_SIZE) as usize;
            let magic = rng.next_u64() as usize;
            allocated += size;

            let block = Box::new(DataBlock::new(id, size, magic));

            // Immediate verification
            if !block.verify(magic) {
                println!("\n[MemWorker] [FAIL] Immediate verification failed for block {} (size {})", id, size);
                return;
            }

            allocations.push((block, magic));
        } else {
            // Deallocate a random block
            let index = (rng.next() as usize) % allocations.len();
            let (block, expected_magic) = allocations.swap_remove(index);

            // Verify integrity before dropping
            if !block.verify(expected_magic) {
                println!("\n[MemWorker] [FAIL] Integrity check failed during mixed test for block {} (size {})", block.id, block.data.len());
                return;
            }
        }

        if i > 0 && i % 50 == 0 {
            println!("[MemWorker] Progress: {}/{} - {}MB", i, TOTAL_OPERATIONS, allocated / 1024 / 1024);
            allocated = 0;
        }
    }

    println!("[MemWorker] Finalizing: verifying remaining {} allocations...", allocations.len());
    for (block, magic) in allocations {
        if !block.verify(magic) {
            println!("[MemWorker] [FAIL] Final verification failed for block {}", block.id);
            return;
        }
    }

    println!("[MemWorker] [PASS] Allocation/Deallocation Stress Test Completed Successfully ({})", total_allocs_performed);
}