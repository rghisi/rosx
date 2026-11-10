use core::alloc::{GlobalAlloc, Layout};
use core::ptr;
use core::sync::atomic::{AtomicBool, Ordering};

use crate::platform::cpu::Cpu;
use crate::kernel_cell::KernelCell;
use crate::memory::MemoryBlocks;
use crate::memory::free_list_allocator::{BlockOwner, FreeListAllocator};

#[cfg_attr(not(test), global_allocator)]
pub(crate) static MEMORY_MANAGER: MemoryManager = MemoryManager::new();

#[cfg_attr(not(test), alloc_error_handler)]
pub fn alloc_error_handler(layout: Layout) -> ! {
    panic!("allocation error: {:?}", layout)
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub(crate) struct MemoryStats {
    pub total_bytes: usize,
    pub free_bytes: usize,
    pub largest_free_block: usize,
    pub free_block_count: usize,
}

pub struct MemoryManager {
    allocator: KernelCell<Option<FreeListAllocator>>,
    is_setup: AtomicBool,
    cpu: KernelCell<Option<&'static dyn Cpu>>,
    memory_blocks: KernelCell<Option<MemoryBlocks>>,
}

impl MemoryManager {
    pub const fn new() -> Self {
        MemoryManager {
            allocator: KernelCell::new(None),
            is_setup: AtomicBool::new(false),
            cpu: KernelCell::new(None),
            memory_blocks: KernelCell::new(None),
        }
    }

    pub fn bootstrap(&self, memory_blocks: &MemoryBlocks) {
        *self.memory_blocks.borrow_mut() = Some(*memory_blocks);
        *self.allocator.borrow_mut() = Some(FreeListAllocator::new(memory_blocks));
    }

    pub fn setup(&self, cpu: &'static dyn Cpu) {
        *self.cpu.borrow_mut() = Some(cpu);
        self.is_setup.store(true, Ordering::SeqCst);
    }

    #[cfg(test)]
    pub(crate) fn ensure_test_memory() {
        static BOOTSTRAPPED: AtomicBool = AtomicBool::new(false);
        if BOOTSTRAPPED.swap(true, Ordering::SeqCst) {
            return;
        }
        let region = alloc::vec![0u8; 64 * 1024 * 1024].leak();
        let blocks = crate::memory::MemoryBlocks {
            blocks: core::array::from_fn(|i| {
                if i == 0 {
                    crate::memory::MemoryBlock { start: region.as_mut_ptr() as usize, size: region.len() }
                } else {
                    crate::memory::MemoryBlock { start: 0, size: 0 }
                }
            }),
            count: 1,
        };
        MEMORY_MANAGER.bootstrap(&blocks);
    }

    pub(crate) fn stats(&self) -> MemoryStats {
        let interrupts_enabled = self.is_setup.load(Ordering::Relaxed)
            && self.cpu.borrow().unwrap().are_interrupts_enabled();
        if interrupts_enabled {
            self.cpu.borrow().unwrap().disable_interrupts();
        }
        let memory_blocks = self.memory_blocks.borrow();
        let memory_blocks = memory_blocks.as_ref().expect("MemoryManager not bootstrapped");
        let mut total_bytes = 0usize;
        for i in 0..memory_blocks.count {
            total_bytes += memory_blocks.blocks[i].size;
        }
        let allocator = self.allocator.borrow();
        let free_list = allocator
            .as_ref()
            .expect("MemoryManager not bootstrapped")
            .stats();
        if interrupts_enabled {
            self.cpu.borrow().unwrap().enable_interrupts();
        }
        MemoryStats {
            total_bytes,
            free_bytes: free_list.free_bytes,
            largest_free_block: free_list.largest_free_block,
            free_block_count: free_list.free_block_count,
        }
    }

    pub(crate) unsafe fn alloc_with_owner(&self, layout: Layout, owner: BlockOwner) -> *mut u8 {
        let interrupts_enabled = self.is_setup.load(Ordering::Relaxed)
            && self.cpu.borrow().unwrap().are_interrupts_enabled();
        if interrupts_enabled {
            self.cpu.borrow().unwrap().disable_interrupts();
        }
        let result = unsafe {
            self.allocator
                .borrow_mut()
                .as_mut()
                .expect("MemoryManager not bootstrapped")
                .allocate(layout, owner)
        };
        if interrupts_enabled {
            self.cpu.borrow().unwrap().enable_interrupts();
        }
        match result {
            Ok(ptr) => ptr,
            Err(_) => ptr::null_mut(),
        }
    }

    pub(crate) fn group_bytes(&self, group_id: usize) -> usize {
        let interrupts_enabled = self.is_setup.load(Ordering::Relaxed)
            && self.cpu.borrow().unwrap().are_interrupts_enabled();
        if interrupts_enabled {
            self.cpu.borrow().unwrap().disable_interrupts();
        }
        let bytes = self
            .allocator
            .borrow()
            .as_ref()
            .expect("MemoryManager not bootstrapped")
            .group_bytes(group_id);
        if interrupts_enabled {
            self.cpu.borrow().unwrap().enable_interrupts();
        }
        bytes
    }

    pub(crate) unsafe fn release_group_allocations(&self, group_id: usize) {
        let interrupts_enabled = self.is_setup.load(Ordering::Relaxed)
            && self.cpu.borrow().unwrap().are_interrupts_enabled();
        if interrupts_enabled {
            self.cpu.borrow().unwrap().disable_interrupts();
        }
        unsafe {
            self.allocator
                .borrow_mut()
                .as_mut()
                .expect("MemoryManager not bootstrapped")
                .deallocate_by_owner(group_id)
        };
        if interrupts_enabled {
            self.cpu.borrow().unwrap().enable_interrupts();
        }
    }

    pub fn print_config(&self) {
        let memory_blocks = self.memory_blocks.borrow();
        let memory_blocks = memory_blocks.as_ref().expect("MemoryManager not bootstrapped");
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
        unsafe { self.alloc_with_owner(layout, BlockOwner::Kernel) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        let interrupts_enabled = self.is_setup.load(Ordering::Relaxed)
            && self.cpu.borrow().unwrap().are_interrupts_enabled();
        if interrupts_enabled {
            self.cpu.borrow().unwrap().disable_interrupts();
        }
        unsafe {
            self.allocator
                .borrow_mut()
                .as_mut()
                .expect("MemoryManager not bootstrapped")
                .deallocate(ptr, layout.align())
        };
        if interrupts_enabled {
            self.cpu.borrow().unwrap().enable_interrupts();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::free_list_allocator::{ALLOC_HDR, BACKREF_BYTES};
    use crate::platform::cpu::Cpu;
    use crate::memory::MemoryBlock;
    use core::alloc::{GlobalAlloc, Layout};

    struct MockCpu;

    impl Cpu for MockCpu {
        fn setup(&self) {}
        fn enable_interrupts(&self) {}
        fn disable_interrupts(&self) {}
        fn are_interrupts_enabled(&self) -> bool { false }
        fn initialize_stack(&self, _: usize, _: usize, _: usize, _: usize) -> usize { 0 }
        fn swap_context(&self, _: *mut usize, _: *mut usize, _: usize) {}
        fn get_system_time(&self) -> u64 { 0 }
        fn halt(&self) {}
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
        manager.bootstrap(&blocks);
        manager
    }

    fn used(manager: &MemoryManager) -> usize {
        let stats = manager.stats();
        stats.total_bytes - stats.free_bytes
    }

    #[test]
    fn used_starts_at_zero() {
        let mut memory = vec![0u8; 1024 * 1024];
        let manager = make_manager(&mut memory);
        assert_eq!(used(&manager), 0);
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
        assert_eq!(used(&manager), 64 + ALLOC_HDR);
        unsafe { manager.dealloc(ptr, layout) };
    }

    #[test]
    fn dealloc_decreases_used() {
        let mut memory = vec![0u8; 1024 * 1024];
        let manager = make_manager(&mut memory);
        let layout = Layout::from_size_align(64, 8).unwrap();
        let ptr = unsafe { manager.alloc(layout) };
        unsafe { manager.dealloc(ptr, layout) };
        assert_eq!(used(&manager), 0);
    }

    #[test]
    fn alloc_with_cpu_setup_returns_non_null() {
        let mut memory = vec![0u8; 1024 * 1024];
        let manager = make_manager(&mut memory);
        manager.setup(&MOCK_CPU);
        let layout = Layout::from_size_align(64, 8).unwrap();
        let ptr = unsafe { manager.alloc(layout) };
        assert!(!ptr.is_null());
        unsafe { manager.dealloc(ptr, layout) };
    }

    #[test]
    fn alloc_with_task_owner_returns_non_null() {
        let mut memory = vec![0u8; 1024 * 1024];
        let manager = make_manager(&mut memory);
        let layout = Layout::from_size_align(64, 8).unwrap();
        let ptr = unsafe { manager.alloc_with_owner(layout, BlockOwner::Group(9)) };
        assert!(!ptr.is_null());
        unsafe { manager.dealloc(ptr, layout) };
    }

    #[test]
    fn alloc_with_task_owner_tracks_used() {
        let mut memory = vec![0u8; 1024 * 1024];
        let manager = make_manager(&mut memory);
        let layout = Layout::from_size_align(128, 8).unwrap();
        let ptr = unsafe { manager.alloc_with_owner(layout, BlockOwner::Group(4)) };
        assert_eq!(used(&manager), 128 + ALLOC_HDR);
        unsafe { manager.dealloc(ptr, layout) };
        assert_eq!(used(&manager), 0);
    }

    #[test]
    fn alloc_with_task_owner_and_cpu_setup_returns_non_null() {
        let mut memory = vec![0u8; 1024 * 1024];
        let manager = make_manager(&mut memory);
        manager.setup(&MOCK_CPU);
        let layout = Layout::from_size_align(64, 8).unwrap();
        let ptr = unsafe { manager.alloc_with_owner(layout, BlockOwner::Group(1)) };
        assert!(!ptr.is_null());
        unsafe { manager.dealloc(ptr, layout) };
    }

    #[test]
    fn alloc_with_owner_exhausted_returns_null() {
        let mut memory = vec![0u8; 128];
        let manager = make_manager(&mut memory);
        let layout = Layout::from_size_align(256, 8).unwrap();
        let ptr = unsafe { manager.alloc_with_owner(layout, BlockOwner::Group(2)) };
        assert!(ptr.is_null());
        assert_eq!(used(&manager), 0);
        let fits = Layout::from_size_align(64, 8).unwrap();
        let ptr = unsafe { manager.alloc_with_owner(fits, BlockOwner::Group(2)) };
        assert!(!ptr.is_null());
        assert_eq!(used(&manager), 64 + ALLOC_HDR);
        unsafe { manager.dealloc(ptr, fits) };
        assert_eq!(used(&manager), 0);
    }

    #[test]
    fn overaligned_alloc_returns_aligned_pointer_and_tracks_used() {
        let mut memory = vec![0u8; 1024 * 1024];
        let manager = make_manager(&mut memory);
        let layout = Layout::from_size_align(64, 16).unwrap();
        let ptr = unsafe { manager.alloc(layout) };
        assert!(!ptr.is_null());
        assert_eq!(ptr as usize % 16, 0);
        assert_eq!(used(&manager), 64 + ALLOC_HDR + 16 + BACKREF_BYTES);
        unsafe { manager.dealloc(ptr, layout) };
        assert_eq!(used(&manager), 0);
    }

    #[test]
    fn release_group_allocations_returns_blocks_to_the_free_list() {
        let mut memory = vec![0u8; 4096];
        let manager = make_manager(&mut memory);
        let small = Layout::from_size_align(64, 8).unwrap();
        let large = Layout::from_size_align(4000, 8).unwrap();
        unsafe { manager.alloc_with_owner(small, BlockOwner::Group(7)) };
        assert!(unsafe { manager.alloc(large) }.is_null());
        unsafe { manager.release_group_allocations(7) };
        assert!(!unsafe { manager.alloc(large) }.is_null());
    }

    #[test]
    fn release_group_allocations_tracks_used() {
        let mut memory = vec![0u8; 1024 * 1024];
        let manager = make_manager(&mut memory);
        let task_layout = Layout::from_size_align(128, 8).unwrap();
        let kernel_layout = Layout::from_size_align(64, 8).unwrap();
        unsafe { manager.alloc_with_owner(task_layout, BlockOwner::Group(3)) };
        let kernel_ptr = unsafe { manager.alloc(kernel_layout) };
        assert_eq!(used(&manager), 128 + 64 + 2 * ALLOC_HDR);
        unsafe { manager.release_group_allocations(3) };
        assert_eq!(used(&manager), 64 + ALLOC_HDR);
        unsafe { manager.dealloc(kernel_ptr, kernel_layout) };
        assert_eq!(used(&manager), 0);
    }

    #[test]
    fn group_release_of_slab_allocation_restores_used_exactly() {
        let mut memory = vec![0u8; 4 * 1024 * 1024];
        let manager = make_manager(&mut memory);
        let kernel_layout = Layout::from_size_align(2 * 1024 * 1024, 8).unwrap();
        let kernel_ptr = unsafe { manager.alloc(kernel_layout) };
        assert_eq!(used(&manager), 2 * 1024 * 1024 + ALLOC_HDR);
        let slab_layout = Layout::from_size_align(65_536, 4_096).unwrap();
        unsafe { manager.alloc_with_owner(slab_layout, BlockOwner::Group(8)) };
        assert_eq!(
            used(&manager),
            2 * 1024 * 1024 + ALLOC_HDR + 65_536 + ALLOC_HDR + 4_096 + BACKREF_BYTES
        );
        unsafe { manager.release_group_allocations(8) };
        assert_eq!(used(&manager), 2 * 1024 * 1024 + ALLOC_HDR);
        unsafe { manager.dealloc(kernel_ptr, kernel_layout) };
        assert_eq!(used(&manager), 0);
    }

    #[test]
    fn group_release_of_unaligned_allocation_restores_used_to_zero() {
        let mut memory = vec![0u8; 1024 * 1024];
        let manager = make_manager(&mut memory);
        let odd = Layout::from_size_align(100, 8).unwrap();
        unsafe { manager.alloc_with_owner(odd, BlockOwner::Group(6)) };
        assert_eq!(used(&manager), 104 + ALLOC_HDR);
        unsafe { manager.release_group_allocations(6) };
        assert_eq!(used(&manager), 0);
    }

    #[test]
    fn release_unknown_task_allocations_changes_nothing() {
        let mut memory = vec![0u8; 1024 * 1024];
        let manager = make_manager(&mut memory);
        let layout = Layout::from_size_align(64, 8).unwrap();
        let ptr = unsafe { manager.alloc_with_owner(layout, BlockOwner::Group(5)) };
        unsafe { manager.release_group_allocations(99) };
        assert_eq!(used(&manager), 64 + ALLOC_HDR);
        unsafe { manager.dealloc(ptr, layout) };
        assert_eq!(used(&manager), 0);
    }

    #[test]
    fn stats_of_fresh_manager_reports_the_full_region_as_free() {
        let mut memory = vec![0u8; 1024 * 1024];
        let manager = make_manager(&mut memory);
        let stats = manager.stats();
        assert_eq!(used(&manager), 0);
        assert_eq!(stats.total_bytes, 1024 * 1024);
        assert_eq!(stats.free_block_count, 1);
        assert!(stats.free_bytes > 0);
        assert!(stats.free_bytes <= stats.total_bytes);
        assert_eq!(stats.largest_free_block, stats.free_bytes);
    }

    #[test]
    fn stats_follow_allocation_and_release() {
        let mut memory = vec![0u8; 1024 * 1024];
        let manager = make_manager(&mut memory);
        let fresh = manager.stats();
        let layout = Layout::from_size_align(128, 8).unwrap();
        let ptr = unsafe { manager.alloc(layout) };
        let after = manager.stats();
        assert_eq!(used(&manager), 128 + ALLOC_HDR);
        assert_eq!(after.total_bytes, fresh.total_bytes);
        assert!(after.free_bytes < fresh.free_bytes);
        assert_eq!(after.free_block_count, fresh.free_block_count);
        assert!(after.largest_free_block < fresh.largest_free_block);
        unsafe { manager.dealloc(ptr, layout) };
        let restored = manager.stats();
        assert_eq!(used(&manager), 0);
        assert_eq!(restored.free_bytes, fresh.free_bytes);
        assert_eq!(restored.free_block_count, fresh.free_block_count);
        assert_eq!(restored.largest_free_block, fresh.largest_free_block);
    }
}
