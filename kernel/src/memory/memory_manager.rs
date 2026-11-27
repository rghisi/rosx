use core::alloc::{GlobalAlloc, Layout};
use core::ptr;
use core::sync::atomic::{AtomicBool, Ordering};

use crate::platform::cpu::Cpu;
use crate::kernel_cell::KernelCell;
use crate::memory::MemoryBlocks;
use crate::memory::free_list_allocator::{BlockOwner, FreeListAllocator};
use crate::memory::slab_heap::{SlabHeap, small_class};

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
    slab: KernelCell<SlabHeap>,
    is_setup: AtomicBool,
    cpu: KernelCell<Option<&'static dyn Cpu>>,
    memory_blocks: KernelCell<Option<MemoryBlocks>>,
}

impl MemoryManager {
    pub const fn new() -> Self {
        MemoryManager {
            allocator: KernelCell::new(None),
            slab: KernelCell::new(SlabHeap::new()),
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
                    crate::memory::MemoryBlock::free(region.as_mut_ptr() as usize, region.len())
                } else {
                    crate::memory::MemoryBlock::free(0, 0)
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

    pub(crate) unsafe fn dealloc_owned(&self, ptr: *mut u8, layout: Layout) {
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

    pub(crate) fn release_held(&self) -> usize {
        let interrupts_enabled = self.is_setup.load(Ordering::Relaxed)
            && self.cpu.borrow().unwrap().are_interrupts_enabled();
        if interrupts_enabled {
            self.cpu.borrow().unwrap().disable_interrupts();
        }
        let returned = self
            .allocator
            .borrow_mut()
            .as_mut()
            .expect("MemoryManager not bootstrapped")
            .release_held();
        if let Some(memory_blocks) = self.memory_blocks.borrow_mut().as_mut() {
            for i in 0..memory_blocks.count {
                if memory_blocks.blocks[i].use_ == crate::memory::BlockUse::Held {
                    memory_blocks.blocks[i].use_ = crate::memory::BlockUse::Free;
                }
            }
        }
        if interrupts_enabled {
            self.cpu.borrow().unwrap().enable_interrupts();
        }
        returned
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
        if small_class(&layout).is_none() {
            return unsafe { self.alloc_with_owner(layout, BlockOwner::Kernel) };
        }
        let interrupts_enabled = self.is_setup.load(Ordering::Relaxed)
            && self.cpu.borrow().unwrap().are_interrupts_enabled();
        if interrupts_enabled {
            self.cpu.borrow().unwrap().disable_interrupts();
        }
        let pointer = {
            let slot = self.allocator.borrow_mut();
            let tier0 = slot
                .as_mut()
                .expect("MemoryManager not bootstrapped");
            unsafe { self.slab.borrow_mut().allocate(tier0, layout) }
        };
        if interrupts_enabled {
            self.cpu.borrow().unwrap().enable_interrupts();
        }
        pointer
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        let interrupts_enabled = self.is_setup.load(Ordering::Relaxed)
            && self.cpu.borrow().unwrap().are_interrupts_enabled();
        if interrupts_enabled {
            self.cpu.borrow().unwrap().disable_interrupts();
        }
        if small_class(&layout).is_some() {
            unsafe { self.slab.borrow_mut().deallocate(ptr, layout) };
        } else {
            unsafe {
                self.allocator
                    .borrow_mut()
                    .as_mut()
                    .expect("MemoryManager not bootstrapped")
                    .deallocate(ptr, layout.align())
            };
        }
        if interrupts_enabled {
            self.cpu.borrow().unwrap().enable_interrupts();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::free_list_allocator::{ALLOC_HDR, BACKREF_BYTES, BLOCK_HDR};
    use crate::memory::slab_heap;
    use crate::platform::cpu::Cpu;
    use crate::memory::{BlockUse, MemoryBlock};
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
                    MemoryBlock::free(memory.as_mut_ptr() as usize, memory.len())
                } else {
                    MemoryBlock::free(0, 0)
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
        let arena_used = used(&manager);
        assert!(arena_used >= slab_heap::ARENA_PAGES * slab_heap::PAGE);
        assert!(arena_used < slab_heap::ARENA_PAGES * slab_heap::PAGE + slab_heap::PAGE);
        unsafe { manager.dealloc(ptr, layout) };
    }

    #[test]
    fn dealloc_of_a_small_kernel_block_retains_the_arena() {
        let mut memory = vec![0u8; 1024 * 1024];
        let manager = make_manager(&mut memory);
        let layout = Layout::from_size_align(64, 8).unwrap();
        let ptr = unsafe { manager.alloc(layout) };
        let with_arena = used(&manager);
        unsafe { manager.dealloc(ptr, layout) };
        assert_eq!(used(&manager), with_arena);
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
        unsafe { manager.dealloc_owned(ptr, layout) };
    }

    #[test]
    fn alloc_with_task_owner_tracks_used() {
        let mut memory = vec![0u8; 1024 * 1024];
        let manager = make_manager(&mut memory);
        let layout = Layout::from_size_align(128, 8).unwrap();
        let ptr = unsafe { manager.alloc_with_owner(layout, BlockOwner::Group(4)) };
        assert_eq!(used(&manager), 128 + ALLOC_HDR);
        unsafe { manager.dealloc_owned(ptr, layout) };
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
        unsafe { manager.dealloc_owned(ptr, layout) };
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
        unsafe { manager.dealloc_owned(ptr, fits) };
        assert_eq!(used(&manager), 0);
    }

    #[test]
    fn overaligned_alloc_returns_aligned_pointer_and_tracks_used() {
        let mut memory = vec![0u8; 1024 * 1024];
        let manager = make_manager(&mut memory);
        let layout = Layout::from_size_align(64, 1024).unwrap();
        let ptr = unsafe { manager.alloc(layout) };
        assert!(!ptr.is_null());
        assert_eq!(ptr as usize % 1024, 0);
        let charge = 64 + ALLOC_HDR + BACKREF_BYTES;
        let overaligned_used = used(&manager);
        assert!(overaligned_used >= charge);
        assert!(overaligned_used < charge + BLOCK_HDR);
        unsafe { manager.dealloc(ptr, layout) };
        assert_eq!(used(&manager), 0);
    }

    #[test]
    fn small_overaligned_alloc_is_served_by_a_wider_slab_class() {
        let mut memory = vec![0u8; 1024 * 1024];
        let manager = make_manager(&mut memory);
        let layout = Layout::from_size_align(64, 64).unwrap();
        let ptr = unsafe { manager.alloc(layout) };
        assert!(!ptr.is_null());
        assert_eq!(ptr as usize % 64, 0);
        assert!(used(&manager) >= slab_heap::ARENA_PAGES * slab_heap::PAGE);
        unsafe { manager.dealloc(ptr, layout) };
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
        let kernel_layout = Layout::from_size_align(1024, 8).unwrap();
        unsafe { manager.alloc_with_owner(task_layout, BlockOwner::Group(3)) };
        let kernel_ptr = unsafe { manager.alloc(kernel_layout) };
        assert_eq!(used(&manager), 128 + 1024 + 2 * ALLOC_HDR);
        unsafe { manager.release_group_allocations(3) };
        assert_eq!(used(&manager), 1024 + ALLOC_HDR);
        unsafe { manager.dealloc(kernel_ptr, kernel_layout) };
        assert_eq!(used(&manager), 0);
    }

    #[test]
    fn kernel_small_blocks_are_invisible_to_group_accounting_and_sweep() {
        let mut memory = vec![0u8; 1024 * 1024];
        let manager = make_manager(&mut memory);
        let layout = Layout::from_size_align(64, 8).unwrap();
        let kernel_ptr = unsafe { manager.alloc(layout) };
        unsafe { manager.alloc_with_owner(layout, BlockOwner::Group(3)) };
        let with_both = used(&manager);
        assert!(with_both >= slab_heap::ARENA_PAGES * slab_heap::PAGE + 64 + ALLOC_HDR);
        assert_eq!(manager.group_bytes(3), 64 + ALLOC_HDR);
        unsafe { manager.release_group_allocations(3) };
        let arena_only = used(&manager);
        assert!(arena_only >= slab_heap::ARENA_PAGES * slab_heap::PAGE);
        assert!(arena_only < slab_heap::ARENA_PAGES * slab_heap::PAGE + slab_heap::PAGE);
        unsafe { manager.dealloc(kernel_ptr, layout) };
        assert_eq!(used(&manager), arena_only);
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
        let slab_charge = 65_536 + ALLOC_HDR + BACKREF_BYTES;
        let with_slab = used(&manager);
        assert!(with_slab >= 2 * 1024 * 1024 + ALLOC_HDR + slab_charge);
        assert!(with_slab < 2 * 1024 * 1024 + ALLOC_HDR + slab_charge + BLOCK_HDR);
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
        unsafe { manager.dealloc_owned(ptr, layout) };
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
        let arena_used = fresh.total_bytes - after.free_bytes;
        assert!(arena_used >= slab_heap::ARENA_PAGES * slab_heap::PAGE);
        assert_eq!(after.total_bytes, fresh.total_bytes);
        assert!(after.largest_free_block < fresh.largest_free_block);
        unsafe { manager.dealloc(ptr, layout) };
        let retained = manager.stats();
        assert_eq!(retained.free_bytes, after.free_bytes);
        assert_eq!(retained.free_block_count, after.free_block_count);
        assert_eq!(retained.largest_free_block, after.largest_free_block);
    }

    #[test]
    fn global_allocator_blocks_are_invisible_to_group_accounting_and_sweep() {
        let mut memory = vec![0u8; 1024 * 1024];
        let manager = make_manager(&mut memory);
        let image_like = Layout::from_size_align(196 * 1024, 1).unwrap();
        unsafe {
            let image = manager.alloc(image_like);
            assert!(!image.is_null());
            let held = used(&manager);
            assert_eq!(held, 196 * 1024 + ALLOC_HDR);

            assert_eq!(manager.group_bytes(0), 0);
            assert_eq!(manager.group_bytes(1), 0);
            manager.release_group_allocations(1);
            assert_eq!(used(&manager), held, "the group sweep must not touch a Kernel-stamped block");

            manager.dealloc(image, image_like);
            assert_eq!(used(&manager), 0);
        }
    }

    #[test]
    fn align_one_allocations_land_on_the_block_alignment_grid() {
        let block_align = core::mem::align_of::<usize>();
        let mut memory = vec![0u8; 1024 * 1024];
        let manager = make_manager(&mut memory);
        let mut blocks = Vec::new();
        unsafe {
            for size in [4096usize, 64 * 1024, 196 * 1024] {
                let layout = Layout::from_size_align(size, 1).unwrap();
                let ptr = manager.alloc(layout);
                assert!(!ptr.is_null());
                assert_eq!(
                    ptr as usize % block_align,
                    0,
                    "a Vec<u8> allocation is guaranteed no better than the {block_align}-byte grid"
                );
                blocks.push((layout, ptr));
            }
            for (layout, ptr) in blocks {
                manager.dealloc(ptr, layout);
            }
        }
        assert_eq!(used(&manager), 0);
    }

    #[test]
    fn small_align_one_allocation_lands_on_its_class_grid() {
        let mut memory = vec![0u8; 1024 * 1024];
        let manager = make_manager(&mut memory);
        let layout = Layout::from_size_align(64, 1).unwrap();
        let ptr = unsafe { manager.alloc(layout) };
        assert!(!ptr.is_null());
        assert_eq!(ptr as usize % 64, 0);
        assert!(used(&manager) >= slab_heap::ARENA_PAGES * slab_heap::PAGE);
        unsafe { manager.dealloc(ptr, layout) };
    }

    #[test]
    fn a_held_block_counts_in_total_but_never_in_free() {
        let mut heap = vec![0u8; 1024 * 1024];
        let mut held = vec![0u8; 64 * 1024];
        let heap_base = heap.as_mut_ptr() as usize;
        let held_base = held.as_mut_ptr() as usize;
        let manager = MemoryManager::new();
        let blocks = MemoryBlocks {
            blocks: core::array::from_fn(|i| match i {
                0 => MemoryBlock::free(heap_base, heap.len()),
                1 => MemoryBlock {
                    start: held_base,
                    size: held.len(),
                    use_: BlockUse::Held,
                },
                _ => MemoryBlock::free(0, 0),
            }),
            count: 2,
        };
        manager.bootstrap(&blocks);
        let stats = manager.stats();
        assert_eq!(stats.total_bytes, heap.len() + held.len());
        let used_now = used(&manager);
        assert!(
            used_now >= held.len() && used_now < held.len() + BLOCK_HDR,
            "the held block reads as used (it is counted in total but never free): used {used_now}"
        );
    }

    #[test]
    fn release_held_returns_the_held_region_to_the_heap_once() {
        let mut heap = vec![0u8; 1024 * 1024];
        let mut held = vec![0u8; 64 * 1024];
        let heap_base = heap.as_mut_ptr() as usize;
        let held_base = held.as_mut_ptr() as usize;
        let manager = MemoryManager::new();
        let blocks = MemoryBlocks {
            blocks: core::array::from_fn(|i| match i {
                0 => MemoryBlock::free(heap_base, heap.len()),
                1 => MemoryBlock {
                    start: held_base,
                    size: held.len(),
                    use_: BlockUse::Held,
                },
                _ => MemoryBlock::free(0, 0),
            }),
            count: 2,
        };
        manager.bootstrap(&blocks);
        let before = manager.stats();
        assert_eq!(manager.release_held(), held.len());
        let after = manager.stats();
        assert_eq!(after.total_bytes, before.total_bytes);
        assert_eq!(after.free_bytes, before.free_bytes + held.len());
        assert_eq!(manager.release_held(), 0, "nothing stays held after the release");
        assert_eq!(manager.stats(), after);
    }
}
