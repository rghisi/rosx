use core::alloc::{GlobalAlloc, Layout};
use core::sync::atomic::{AtomicUsize, Ordering};

use buddy_system_allocator::LockedHeap;

use crate::cpu::Cpu;
use crate::kernel_cell::KernelCell;

#[cfg_attr(not(test), global_allocator)]
pub(crate) static MEMORY_MANAGER: MemoryManager = MemoryManager::new();
const MAX_MEMORY_BLOCKS: usize = 32;

pub struct MemoryBlock {
    pub start: usize,
    pub size: usize,
}

pub struct MemoryBlocks {
    pub blocks: [MemoryBlock; MAX_MEMORY_BLOCKS],
    pub count: usize,
}

#[cfg_attr(not(test), alloc_error_handler)]
pub fn alloc_error_handler(layout: Layout) -> ! {
    panic!("allocation error: {:?}", layout)
}

pub struct MemoryManager {
    heap: LockedHeap<27>,
    used: AtomicUsize,
    cpu: KernelCell<Option<&'static dyn Cpu>>,
}

impl MemoryManager {
    pub const fn new() -> Self {
        MemoryManager {
            heap: LockedHeap::new(),
            used: AtomicUsize::new(0),
            cpu: KernelCell::new(None),
        }
    }

    pub fn init(&self, memory_blocks: &MemoryBlocks) {
        for i in 0..memory_blocks.count {
            let block = &memory_blocks.blocks[i];
            unsafe {
                self.heap.lock().init(block.start, block.size);
            }
        }
    }

    pub fn set_cpu(&self, cpu: &'static dyn Cpu) {
        *self.cpu.borrow_mut() = Some(cpu);
    }

    pub fn used(&self) -> usize {
        self.used.load(Ordering::Relaxed)
    }

    pub fn print_config(&self, memory_blocks: &MemoryBlocks) {
        let mut total_size: usize = 0;
        for i in 0..memory_blocks.count {
            let block = &memory_blocks.blocks[i];
            let end = block.start + block.size;
            crate::kprintln!(
                "[MEMORY] Region: 0x{:x}-0x{:x} ({} KB)",
                block.start,
                end,
                block.size / 1024
            );
            total_size += block.size;
        }
        crate::kprintln!("[MEMORY] Total: {} MB", total_size / (1024 * 1024));
    }
}

unsafe impl GlobalAlloc for MemoryManager {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let cpu = *self.cpu.borrow();
        let interrupts_enabled = cpu.map_or(false, |c| c.are_interrupts_enabled());
        if interrupts_enabled {
            cpu.unwrap().disable_interrupts();
        }
        self.used.fetch_add(layout.size(), Ordering::Relaxed);
        let ptr = unsafe { self.heap.alloc(layout) };
        if interrupts_enabled {
            cpu.unwrap().enable_interrupts();
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        let cpu = *self.cpu.borrow();
        let interrupts_enabled = cpu.map_or(false, |c| c.are_interrupts_enabled());
        if interrupts_enabled {
            cpu.unwrap().disable_interrupts();
        }
        unsafe { self.heap.dealloc(ptr, layout) };
        self.used.fetch_sub(layout.size(), Ordering::Relaxed);
        if interrupts_enabled {
            cpu.unwrap().enable_interrupts();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cpu::Cpu;
    use core::alloc::{GlobalAlloc, Layout};

    struct MockCpu;

    impl Cpu for MockCpu {
        fn setup(&self) {}
        fn enable_interrupts(&self) {}
        fn disable_interrupts(&self) {}
        fn are_interrupts_enabled(&self) -> bool { false }
        fn initialize_stack(&self, _: usize, _: usize, _: usize, _: usize) -> usize { 0 }
        fn swap_context(&self, _: *mut usize, _: usize) {}
        fn get_system_time(&self) -> u64 { 0 }
    }

    static MOCK_CPU: MockCpu = MockCpu;

    fn make_manager(memory: &mut Vec<u8>) -> MemoryManager {
        let manager = MemoryManager::new();
        let blocks = MemoryBlocks {
            blocks: core::array::from_fn(|i| {
                if i == 0 {
                    MemoryBlock { start: memory.as_mut_ptr() as usize, size: memory.len() }
                } else {
                    MemoryBlock { start: 0, size: 0 }
                }
            }),
            count: 1,
        };
        manager.init(&blocks);
        manager
    }

    #[test]
    fn used_starts_at_zero() {
        let mut memory = vec![0u8; 1024 * 1024];
        let manager = make_manager(&mut memory);
        assert_eq!(manager.used(), 0);
    }

    #[test]
    fn alloc_returns_non_null() {
        let mut memory = vec![0u8; 1024 * 1024];
        let manager = make_manager(&mut memory);
        let layout = Layout::from_size_align(64, 8).unwrap();
        let ptr = unsafe { manager.alloc(layout) };
        assert!(!ptr.is_null());
        unsafe { manager.dealloc(ptr, layout) };
    }

    #[test]
    fn alloc_increases_used() {
        let mut memory = vec![0u8; 1024 * 1024];
        let manager = make_manager(&mut memory);
        let layout = Layout::from_size_align(64, 8).unwrap();
        let ptr = unsafe { manager.alloc(layout) };
        assert_eq!(manager.used(), 64);
        unsafe { manager.dealloc(ptr, layout) };
    }

    #[test]
    fn dealloc_decreases_used() {
        let mut memory = vec![0u8; 1024 * 1024];
        let manager = make_manager(&mut memory);
        let layout = Layout::from_size_align(64, 8).unwrap();
        let ptr = unsafe { manager.alloc(layout) };
        unsafe { manager.dealloc(ptr, layout) };
        assert_eq!(manager.used(), 0);
    }

    #[test]
    fn alloc_with_cpu_set_returns_non_null() {
        let mut memory = vec![0u8; 1024 * 1024];
        let manager = make_manager(&mut memory);
        manager.set_cpu(&MOCK_CPU);
        let layout = Layout::from_size_align(64, 8).unwrap();
        let ptr = unsafe { manager.alloc(layout) };
        assert!(!ptr.is_null());
        unsafe { manager.dealloc(ptr, layout) };
    }
}
