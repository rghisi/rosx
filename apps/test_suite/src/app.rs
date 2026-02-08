use alloc::vec::Vec;
use alloc::boxed::Box;
use alloc::string::String;
use alloc::format;
use usrlib::{print, println};
use usrlib::syscall::Syscall;

const CONTEXT_SWITCH_ITERATIONS: usize = 50;

struct SimpleRng {
    state: u32,
}

impl SimpleRng {
    fn new(seed: u32) -> Self {
        SimpleRng { state: seed }
    }

    fn next(&mut self) -> u32 {
        self.state = self.state.wrapping_mul(1103515245).wrapping_add(12345);
        self.state
    }

    fn next_u64(&mut self) -> u64 {
        let hi = self.next() as u64;
        let lo = self.next() as u64;
        (hi << 32) | lo
    }

    fn next_range(&mut self, min: u32, max: u32) -> u32 {
        let range = max - min + 1;
        min + (self.next() % range)
    }
}

struct DataBlock {
    magic: u64,
    id: usize,
    data: Vec<u8>,
}

impl DataBlock {
    fn new(id: usize, size: usize, magic: u64) -> Self {
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

    fn verify(&self, expected_magic: u64) -> bool {
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

pub fn main() {
    println!("=== RosX Test Suite Started ===");
    println!("Spawning worker tasks...");

    // Spawn concurrent workers
    Syscall::exec(worker_memory_stress as usize);
    // Syscall::exec(worker_context_switch as usize);
    // Syscall::exec(worker_mixed_load as usize);
    //
    // // Main thread also does some work to test multitasking
    // println!("[Main] Performing main thread checks...");
    // for i in 0..5 {
    //     Syscall::sleep(500);
    //     println!("[Main] iteration {}", i);
    // }
    
    println!("=== Main Thread Finished ===");
    // In a real OS, we might wait for children here, but for now we just exit/loop
    loop {
        Syscall::sleep(1000);
    }
}

pub fn worker_memory_stress() {
    println!("[MemWorker] Starting Mixed Allocation/Deallocation Stress Test...");
    
    let mut allocations: Vec<(Box<DataBlock>, u64)> = Vec::new();
    let mut rng = SimpleRng::new(0x1337);
    let mut total_allocs_performed = 0;
    
    const MAX_CONCURRENT_ALLOCS: usize = 1000;
    const TOTAL_OPERATIONS: usize = 2000000;

    for i in 0..TOTAL_OPERATIONS {
        // Decide: Allocate or Deallocate?
        let should_allocate = if allocations.len() >= MAX_CONCURRENT_ALLOCS {
            false // Forced deallocate if at "OOM" limit
        } else if allocations.is_empty() {
            true  // Forced allocate if empty
        } else {
            (rng.next() % 2) == 0
        };

        if should_allocate {
            let id = total_allocs_performed;
            total_allocs_performed += 1;
            let size = rng.next_range(16, 20480) as usize;
            let magic = rng.next_u64();

            let block = Box::new(DataBlock::new(id, size, magic));
            
            // Immediate verification
            if !block.verify(magic) {
                println!("\n[MemWorker] [FAIL] Immediate verification failed for block {} (size {})", id, size);
                return;
            }
            
            allocations.push((block, magic));
            
            if total_allocs_performed % 100 == 0 {
                print!("+"); // Progress indicator for allocation
            }
        } else {
            // Deallocate a random block
            let index = (rng.next() as usize) % allocations.len();
            let (block, expected_magic) = allocations.swap_remove(index);
            
            // Verify integrity before dropping
            if !block.verify(expected_magic) {
                println!("\n[MemWorker] [FAIL] Integrity check failed during mixed test for block {} (size {})", block.id, block.data.len());
                return;
            }
            
            if i % 100 == 0 {
                print!("-"); // Progress indicator for deallocation
            }
        }

        // Periodically yield to let other tasks run
        // if i % 10 == 0 {
        //     Syscall::task_yield();
        // }
    }
    
    println!("\n[MemWorker] Finalizing: verifying remaining {} allocations...", allocations.len());
    for (block, magic) in allocations {
        if !block.verify(magic) {
            println!("[MemWorker] [FAIL] Final verification failed for block {}", block.id);
            return;
        }
    }
    
    println!("[MemWorker] [PASS] Mixed Stress Test Completed Successfully ({})", total_allocs_performed);
}

pub fn worker_context_switch() {
    println!("[CtxWorker] Starting Context Switch Test...");
    
    for i in 0..CONTEXT_SWITCH_ITERATIONS {
        if i % 10 == 0 {
             print!("."); // distinct visual indicator
        }
        // Heavy yielding to force context switches
        Syscall::task_yield();
    }
    println!("");
    println!("[CtxWorker] [PASS] Context Switch Test Completed");
}

pub fn worker_mixed_load() {
    println!("[MixWorker] Starting Mixed Load Test...");
    
    let mut s = String::from("Start");
    
    for i in 0..10 {
        s.push_str(&format!("-{}", i));
        Syscall::sleep(100); // Sleep causes scheduling
        
        // Check string integrity implicitly by length and content if possible
        // Ideally we'd verify the string content here
    }
    
    if s.starts_with("Start-0-1") {
         println!("[MixWorker] [PASS] String manipulation and sleep test passed");
    } else {
         println!("[MixWorker] [FAIL] String content unexpected: {}", s);
    }
}
