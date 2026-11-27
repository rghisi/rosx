use core::alloc::Layout;
use core::ptr::NonNull;

use crate::memory::free_list_allocator::BlockOwner;
use crate::memory::memory_manager::MEMORY_MANAGER;
use crate::task::GroupId;

#[cfg(test)]
use crate::memory::memory_manager::MemoryManager;

pub(crate) struct ProgramMemory {
    pointer: NonNull<u8>,
    len: usize,
}

impl ProgramMemory {
    /// Stamps the zeroed block `BlockOwner::Group(group)`; it is freed only by the group's
    /// last-member sweep, never by this type. See docs/rationale/group-stamped-program-memory.md.
    pub(crate) fn allocate(group: GroupId, len: usize, align: usize) -> Option<ProgramMemory> {
        #[cfg(test)]
        <MemoryManager>::ensure_test_memory();
        let layout = Layout::from_size_align(len, align).ok()?;
        let pointer = unsafe {
            let block = MEMORY_MANAGER.alloc_with_owner(layout, BlockOwner::Group(group.value()));
            if !block.is_null() {
                core::slice::from_raw_parts_mut(block, len).fill(0);
            }
            block
        };
        NonNull::new(pointer).map(|pointer| ProgramMemory { pointer, len })
    }

    /// The block belongs to the allocator until the group sweep frees it; the view aliases it.
    /// See docs/rationale/group-stamped-program-memory.md.
    pub(crate) fn bytes_mut(&mut self) -> &mut [u8] {
        unsafe { core::slice::from_raw_parts_mut(self.pointer.as_ptr(), self.len) }
    }

    pub(crate) fn base(&self) -> usize {
        self.pointer.addr().get()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allocate_stamps_group_owned_zeroed_page_aligned_memory() {
        let group = GroupId::allocate();
        assert_eq!(MEMORY_MANAGER.group_bytes(group.value()), 0);

        let mut program = ProgramMemory::allocate(group, 8192, 4096).expect("allocate program memory");

        assert_eq!(program.base() % 4096, 0);
        assert!(program.bytes_mut().iter().all(|byte| *byte == 0));
        assert!(MEMORY_MANAGER.group_bytes(group.value()) >= 8192);
    }

    #[test]
    fn the_group_sweep_reclaims_program_memory_and_writes_to_it() {
        let group = GroupId::allocate();
        let mut program = ProgramMemory::allocate(group, 4096, 16).expect("allocate program memory");
        program.bytes_mut()[0] = 0x5A;
        let swept = MEMORY_MANAGER.group_bytes(group.value());
        assert!(swept > 0);

        unsafe { MEMORY_MANAGER.release_group_allocations(group.value()) };

        assert_eq!(MEMORY_MANAGER.group_bytes(group.value()), 0);
    }

    #[test]
    fn an_allocation_larger_than_the_region_returns_none() {
        let group = GroupId::allocate();
        assert!(ProgramMemory::allocate(group, 1usize << 48, 4096).is_none());
        assert_eq!(MEMORY_MANAGER.group_bytes(group.value()), 0);
    }
}
