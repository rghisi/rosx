use core::alloc::{GlobalAlloc, Layout};
use core::ptr::NonNull;

use crate::memory::free_list_allocator::BlockOwner;
use crate::memory::memory_manager::MEMORY_MANAGER;
#[cfg(test)]
use crate::memory::memory_manager::MemoryManager;
use crate::task::GroupId;

#[cfg(target_pointer_width = "32")]
pub const STACK_WORDS: usize = 12288;
#[cfg(target_pointer_width = "64")]
pub const STACK_WORDS: usize = 8192;

#[cfg(target_pointer_width = "32")]
pub const STACK_WATERMARK: usize = 0xDEAD_BEEF;
#[cfg(target_pointer_width = "64")]
pub const STACK_WATERMARK: usize = 0xDEAD_BEEF_DEAD_BEEF;

pub(crate) const STACK_GUARD_BYTES: usize = 128;

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub(crate) enum StackObservation {
    WithinBudget,
    SpillIntoGuard,
    Exhausted,
}

pub struct TaskStack {
    pointer: NonNull<usize>,
    deep_watermark: usize,
}

impl TaskStack {
    pub fn new(group: GroupId) -> TaskStack {
        #[cfg(test)]
        MemoryManager::ensure_test_memory();
        let pointer = unsafe {
            MEMORY_MANAGER.alloc_with_owner(Self::layout(), BlockOwner::Stack(group.value()))
        };
        if pointer.is_null() {
            panic!("cannot allocate a {} KiB task stack", Self::size_bytes_static() / 1024);
        }
        let stack = TaskStack {
            pointer: unsafe { NonNull::new_unchecked(pointer as *mut usize) },
            deep_watermark: 0,
        };
        unsafe {
            core::slice::from_raw_parts_mut(stack.pointer.as_ptr(), STACK_WORDS).fill(STACK_WATERMARK);
        }
        stack
    }

    fn layout() -> Layout {
        Layout::array::<usize>(STACK_WORDS).unwrap()
    }

    fn size_bytes_static() -> usize {
        STACK_WORDS * core::mem::size_of::<usize>()
    }

    pub fn top(&self) -> usize {
        self.base() + self.size_bytes()
    }

    pub(crate) fn base(&self) -> usize {
        self.pointer.as_ptr().addr()
    }

    pub(crate) fn size_bytes(&self) -> usize {
        Self::size_bytes_static()
    }

    pub(crate) fn observe_depth(&mut self) -> usize {
        let mut depth = self.deep_watermark;
        while depth < STACK_WORDS && self.words()[STACK_WORDS - 1 - depth] != STACK_WATERMARK {
            depth += 1;
        }
        self.deep_watermark = depth;
        depth
    }

    pub(crate) fn is_exhausted(&self) -> bool {
        self.deep_watermark == STACK_WORDS
    }

    pub(crate) fn words(&self) -> &[usize] {
        unsafe { core::slice::from_raw_parts(self.pointer.as_ptr(), STACK_WORDS) }
    }

    #[cfg(test)]
    pub(crate) fn set_word(&mut self, index: usize, value: usize) {
        unsafe { self.pointer.as_ptr().add(index).write(value) };
    }
}

impl Drop for TaskStack {
    fn drop(&mut self) {
        unsafe { MEMORY_MANAGER.dealloc(self.pointer.as_ptr() as *mut u8, Self::layout()) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn top_is_one_past_the_top() {
        let stack = TaskStack::new(GroupId::allocate());
        assert_eq!(stack.top(), stack.base() + stack.size_bytes());
    }

    #[test]
    fn buffer_is_watermark_filled() {
        let stack = TaskStack::new(GroupId::allocate());
        assert!(stack.words().iter().all(|w| *w == STACK_WATERMARK));
    }

    #[test]
    fn addresses_are_nonzero() {
        let stack = TaskStack::new(GroupId::allocate());
        assert_ne!(stack.base(), 0);
        assert_ne!(stack.top(), 0);
    }

    #[test]
    fn fresh_stack_observes_zero_depth() {
        let mut stack = TaskStack::new(GroupId::allocate());
        assert_eq!(stack.observe_depth(), 0);
        assert!(!stack.is_exhausted());
    }

    #[test]
    fn depth_counts_contiguous_frames_from_the_top() {
        let mut stack = TaskStack::new(GroupId::allocate());
        stack.set_word(STACK_WORDS - 1, 0x1);
        stack.set_word(STACK_WORDS - 2, 0x2);
        assert_eq!(stack.observe_depth(), 2);
    }

    #[test]
    fn depth_never_shrinks_once_observed() {
        let mut stack = TaskStack::new(GroupId::allocate());
        stack.set_word(STACK_WORDS - 1, 0x1);
        assert_eq!(stack.observe_depth(), 1);
        stack.set_word(STACK_WORDS - 1, STACK_WATERMARK);
        assert_eq!(stack.observe_depth(), 1);
    }

    #[test]
    fn depth_stops_at_watermark_holes() {
        let mut stack = TaskStack::new(GroupId::allocate());
        stack.set_word(STACK_WORDS - 1, 0x1);
        stack.set_word(STACK_WORDS - 3, 0x3);
        assert_eq!(stack.observe_depth(), 1);
    }

    #[test]
    fn full_buffer_is_exhausted() {
        let mut stack = TaskStack::new(GroupId::allocate());
        for i in 0..STACK_WORDS {
            stack.set_word(i, i + 1);
        }
        assert_eq!(stack.observe_depth(), STACK_WORDS);
        assert!(stack.is_exhausted());
    }
}
