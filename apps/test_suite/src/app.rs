use alloc::vec::Vec;
use alloc::boxed::Box;
use alloc::string::String;
use alloc::format;
use usrlib::{print, println};
use usrlib::syscall::Syscall;

const MEMORY_TEST_ITERATIONS: usize = 10;
const CONTEXT_SWITCH_ITERATIONS: usize = 50;

// Magic number to identify our data blocks
const MAGIC_NUMBER: u64 = 0xDEAD_BEEF_CAFE_BABE;

struct DataBlock {
    magic: u64,
    id: usize,
    data: [u8; 32],
}

impl DataBlock {
    fn new(id: usize) -> Self {
        let mut data = [0u8; 32];
        for i in 0..32 {
            data[i] = (id as u8).wrapping_add(i as u8);
        }
        DataBlock {
            magic: MAGIC_NUMBER,
            id,
            data,
        }
    }

    fn verify(&self) -> bool {
        if self.magic != MAGIC_NUMBER {
            println!("Memory Corruption: Magic number mismatch! Expected: {:x}, Found: {:x}", MAGIC_NUMBER, self.magic);
            return false;
        }
        for i in 0..32 {
            if self.data[i] != (self.id as u8).wrapping_add(i as u8) {
                println!("Memory Corruption: Data mismatch at index {}! Expected: {}, Found: {}", i, (self.id as u8).wrapping_add(i as u8), self.data[i]);
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
    Syscall::exec(worker_context_switch as usize);
    Syscall::exec(worker_mixed_load as usize);

    // Main thread also does some work to test multitasking
    println!("[Main] Performing main thread checks...");
    for i in 0..5 {
        Syscall::sleep(500);
        println!("[Main] iteration {}", i);
    }
    
    println!("=== Main Thread Finished ===");
    // In a real OS, we might wait for children here, but for now we just exit/loop
    loop {
        Syscall::sleep(1000);
    }
}

pub fn worker_memory_stress() {
    println!("[MemWorker] Starting Memory Stress Test...");
    
    let mut allocations = Vec::new();
    
    for i in 0..MEMORY_TEST_ITERATIONS {
        // Allocate
        let block = Box::new(DataBlock::new(i));
        
        // Verify immediately
        if !block.verify() {
            println!("[MemWorker] [FAIL] Immediate verification failed for block {}", i);
            return;
        }
        
        allocations.push(block);
        
        // Yield to allow other tasks to run and potentially corrupt memory if unsafe
        Syscall::task_yield();
    }
    
    println!("[MemWorker] Verifying all {} allocations...", allocations.len());
    
    // Verify all allocations after some time
    for (i, block) in allocations.iter().enumerate() {
        if !block.verify() {
            println!("[MemWorker] [FAIL] Delayed verification failed for block {}", i);
            return;
        }
    }
    
    // Clean up happens when `allocations` goes out of scope
    println!("[MemWorker] [PASS] Memory Stress Test Completed Successfully");
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
