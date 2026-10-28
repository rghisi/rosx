use crate::allocation_test::pattern_byte;
use crate::random::SimpleRng;
use alloc::alloc::{alloc, dealloc};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::alloc::Layout;
use usrlib::println;

const ALIGNMENTS: [usize; 10] = [8, 16, 32, 64, 128, 256, 512, 1024, 2048, 4096];
const MAX_CONCURRENT_BUFFERS: usize = 8;
const TOTAL_OPERATIONS: usize = 2000;
const SWEEP_INTERVAL: usize = 50;

const MINIMUM_SIZE: usize = 16;
const MAXIMUM_SIZE: usize = 64 * 1024;

struct AlignedBuffer {
    id: usize,
    ptr: *mut u8,
    layout: Layout,
    key: u64,
}

impl AlignedBuffer {
    fn new(id: usize, layout: Layout, key: u64) -> Option<Self> {
        // SAFETY: `layout` is a non-zero size with a power-of-two alignment, so
        // it is a valid allocation request. The returned block is either null
        // or exclusively owned by this buffer until `release` frees it once.
        let ptr = unsafe { alloc(layout) };
        if ptr.is_null() {
            return None;
        }
        // SAFETY: `ptr` points to a live allocation of `layout.size()` bytes.
        let payload = unsafe { core::slice::from_raw_parts_mut(ptr, layout.size()) };
        for (index, byte) in payload.iter_mut().enumerate() {
            *byte = pattern_byte(key, index);
        }
        Some(AlignedBuffer { id, ptr, layout, key })
    }

    fn misalignment(&self) -> Option<String> {
        let address = self.ptr as usize;
        let offset = address % self.layout.align();
        if offset == 0 {
            return None;
        }
        Some(format!(
            "pointer {:#x} is {} bytes off {}-byte alignment",
            address,
            offset,
            self.layout.align()
        ))
    }

    fn verify_payload(&self) -> Option<String> {
        // SAFETY: `ptr` is live for `layout.size()` bytes until `release`, and
        // this method only reads within that range.
        let payload = unsafe { core::slice::from_raw_parts(self.ptr, self.layout.size()) };
        for (index, &byte) in payload.iter().enumerate() {
            let expected = pattern_byte(self.key, index);
            if byte != expected {
                return Some(format!(
                    "payload mismatch at index {} (expected 0x{:02x}, found 0x{:02x})",
                    index, expected, byte
                ));
            }
        }
        None
    }

    fn release(self) -> Result<(), String> {
        if let Some(reason) = self.verify_payload() {
            return Err(format!(
                "block {} ({} bytes at {}-byte align): {}",
                self.id,
                self.layout.size(),
                self.layout.align(),
                reason
            ));
        }
        // SAFETY: `ptr` came from `alloc` with this exact layout and each
        // buffer reaches `release` at most once.
        unsafe { dealloc(self.ptr, self.layout) };
        Ok(())
    }
}

fn sweep(buffers: &[AlignedBuffer]) -> Option<(usize, String)> {
    for buffer in buffers {
        if let Some(reason) = buffer.misalignment().or_else(|| buffer.verify_payload()) {
            return Some((buffer.id, reason));
        }
    }
    None
}

fn random_layout(rng: &mut SimpleRng) -> Layout {
    let align = ALIGNMENTS[rng.next_range(0, (ALIGNMENTS.len() - 1) as u32) as usize];
    let size = rng.next_range(MINIMUM_SIZE as u32, MAXIMUM_SIZE as u32) as usize;
    Layout::from_size_align(size, align).unwrap()
}

pub fn run() {
    println!("[AlignWorker] Starting Alignment Stress Test...");

    let mut buffers: Vec<AlignedBuffer> = Vec::new();
    let mut rng = SimpleRng::new(0xA11CE);
    let mut total_allocs_performed = 0;

    for operation in 1..=TOTAL_OPERATIONS {
        let should_allocate = if buffers.len() >= MAX_CONCURRENT_BUFFERS {
            false
        } else if buffers.is_empty() {
            true
        } else {
            rng.coin()
        };

        if should_allocate {
            let layout = random_layout(&mut rng);
            let id = total_allocs_performed;
            let buffer = match AlignedBuffer::new(id, layout, rng.next_u64()) {
                Some(buffer) => buffer,
                None => {
                    println!(
                        "[AlignWorker] [FAIL] allocator refused {} bytes at {}-byte alignment",
                        layout.size(),
                        layout.align()
                    );
                    return;
                }
            };
            if let Some(reason) = buffer.misalignment() {
                println!(
                    "[AlignWorker] [FAIL] block {} ({} bytes): {}",
                    id,
                    layout.size(),
                    reason
                );
                return;
            }
            buffers.push(buffer);
            total_allocs_performed += 1;
        } else {
            let index = rng.next_range(0, (buffers.len() - 1) as u32) as usize;
            let buffer = buffers.swap_remove(index);
            if let Err(reason) = buffer.release() {
                println!("[AlignWorker] [FAIL] on release: {}", reason);
                return;
            }
        }

        if operation % SWEEP_INTERVAL == 0 {
            if let Some((id, reason)) = sweep(&buffers) {
                println!(
                    "[AlignWorker] [FAIL] sweep at operation {}: block {}: {}",
                    operation, id, reason
                );
                return;
            }
            println!(
                "[AlignWorker] Progress: {}/{} - {} live buffers",
                operation,
                TOTAL_OPERATIONS,
                buffers.len()
            );
        }
    }

    println!(
        "[AlignWorker] Finalizing: releasing {} live buffers...",
        buffers.len()
    );
    for buffer in buffers.drain(..) {
        if let Err(reason) = buffer.release() {
            println!("[AlignWorker] [FAIL] on final release: {}", reason);
            return;
        }
    }

    println!(
        "[AlignWorker] [PASS] Alignment Stress Test Completed Successfully ({})",
        total_allocs_performed
    );
}
