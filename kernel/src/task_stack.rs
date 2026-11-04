use crate::memory::memory_manager::alloc_error_handler;
use alloc::alloc::{alloc_zeroed, dealloc};
use core::alloc::Layout;
use core::fmt::{Display, Formatter};
use core::ptr::NonNull;
use system::stack::STACK_GUARD_MAGIC;

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum StackKind {
    KernelFunction,
    ElfUser,
    Scheduler,
    Idle,
}

impl StackKind {
    pub const fn body_bytes(&self) -> usize {
        match self {
            StackKind::ElfUser => 64 * 1024,
            StackKind::KernelFunction => 16 * 1024,
            StackKind::Scheduler => 16 * 1024,
            StackKind::Idle => 8 * 1024,
        }
    }

    pub(crate) const fn body_words(&self) -> usize {
        self.body_bytes() / core::mem::size_of::<usize>()
    }
}

impl Display for StackKind {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        match self {
            StackKind::KernelFunction => write!(f, "KernelFunction"),
            StackKind::ElfUser => write!(f, "ElfUser"),
            StackKind::Scheduler => write!(f, "Scheduler"),
            StackKind::Idle => write!(f, "Idle"),
        }
    }
}

#[cfg(target_pointer_width = "32")]
pub const STACK_WATERMARK: usize = 0xDEAD_BEEF;
#[cfg(target_pointer_width = "64")]
pub const STACK_WATERMARK: usize = 0xDEAD_BEEF_DEAD_BEEF;

pub(crate) const STACK_FLOOR_BYTES: usize = 128;

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub(crate) enum StackObservation {
    WithinBudget,
    SpillIntoFloor,
    Exhausted,
}

fn align_up(value: usize, alignment: usize) -> usize {
    (value + alignment - 1) & !(alignment - 1)
}

pub struct TaskStack {
    region: NonNull<u8>,
    region_bytes: usize,
    base: usize,
    kind: StackKind,
    deep_watermark: usize,
}

impl TaskStack {
    pub fn new(kind: StackKind) -> TaskStack {
        let word_bytes = core::mem::size_of::<usize>();
        let region_bytes = 2 * kind.body_bytes() + word_bytes;
        let layout =
            Layout::from_size_align(region_bytes, word_bytes).unwrap_or_else(|_| unreachable!());
        // SAFETY: the layout is a valid non-zero size with power-of-two alignment.
        // A null pointer means the kernel heap is exhausted.
        let region = unsafe { alloc_zeroed(layout) };
        if region.is_null() {
            alloc_error_handler(layout);
        }
        // SAFETY: region was checked non-null immediately above.
        let region = unsafe { NonNull::new_unchecked(region) };
        let base = align_up(region.as_ptr() as usize + word_bytes, kind.body_bytes());
        // SAFETY: the aligned base lies at most word_bytes + body_bytes - 1
        // above the region start, so the guard word at base - word_bytes and
        // the full aligned body fit inside the zero-filled region.
        unsafe {
            let body = core::slice::from_raw_parts_mut(base as *mut usize, kind.body_words());
            for word in body.iter_mut() {
                *word = STACK_WATERMARK;
            }
            core::ptr::write((base - word_bytes) as *mut usize, STACK_GUARD_MAGIC);
        }
        TaskStack {
            region,
            region_bytes,
            base,
            kind,
            deep_watermark: 0,
        }
    }

    pub fn top(&self) -> usize {
        self.base + self.size_bytes()
    }

    pub(crate) fn base(&self) -> usize {
        self.base
    }

    pub(crate) fn kind(&self) -> StackKind {
        self.kind
    }

    pub(crate) fn size_bytes(&self) -> usize {
        self.kind.body_bytes()
    }

    pub(crate) fn guard_address(&self) -> usize {
        self.base - core::mem::size_of::<usize>()
    }

    pub(crate) fn guard_word(&self) -> usize {
        // SAFETY: the guard word was written at construction and lives inside
        // the owned region for the whole lifetime of self.
        unsafe { core::ptr::read(self.guard_address() as *const usize) }
    }

    pub(crate) fn guard_intact(&self) -> bool {
        self.guard_word() == STACK_GUARD_MAGIC
    }

    fn body(&self) -> &[usize] {
        // SAFETY: the body spans the aligned range new() filled, which outlives self.
        unsafe { core::slice::from_raw_parts(self.base as *const usize, self.kind.body_words()) }
    }

    pub(crate) fn observe_depth(&mut self) -> usize {
        let body = self.body();
        let mut depth = self.deep_watermark;
        while depth < body.len() && body[body.len() - 1 - depth] != STACK_WATERMARK {
            depth += 1;
        }
        self.deep_watermark = depth;
        depth
    }

    pub(crate) fn is_exhausted(&self) -> bool {
        self.deep_watermark == self.kind.body_words()
    }

    #[cfg(test)]
    pub(crate) fn words(&self) -> &[usize] {
        self.body()
    }

    #[cfg(test)]
    pub(crate) fn set_word(&mut self, index: usize, value: usize) {
        // SAFETY: test callers keep index inside the body.
        unsafe {
            let address = self.base + index * core::mem::size_of::<usize>();
            core::ptr::write(address as *mut usize, value);
        }
    }

    #[cfg(test)]
    pub(crate) fn set_guard_word(&mut self, value: usize) {
        // SAFETY: the guard word lives inside the owned region.
        unsafe { core::ptr::write(self.guard_address() as *mut usize, value) };
    }
}

impl Drop for TaskStack {
    fn drop(&mut self) {
        let layout = Layout::from_size_align(self.region_bytes, core::mem::size_of::<usize>())
            .unwrap_or_else(|_| unreachable!());
        // SAFETY: region was allocated by new() with exactly this layout, and
        // no borrow of the body or guard word can outlive self.
        unsafe { dealloc(self.region.as_ptr(), layout) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL_KINDS: [StackKind; 4] = [
        StackKind::KernelFunction,
        StackKind::ElfUser,
        StackKind::Scheduler,
        StackKind::Idle,
    ];

    #[test]
    fn body_sizes_are_named_byte_budgets() {
        assert_eq!(StackKind::ElfUser.body_bytes(), 64 * 1024);
        assert_eq!(StackKind::KernelFunction.body_bytes(), 16 * 1024);
        assert_eq!(StackKind::Scheduler.body_bytes(), 16 * 1024);
        assert_eq!(StackKind::Idle.body_bytes(), 8 * 1024);
    }

    #[test]
    fn every_kind_is_power_of_two_and_aligned_to_its_own_size() {
        for kind in ALL_KINDS {
            let stack = TaskStack::new(kind);
            assert!(kind.body_bytes().is_power_of_two());
            assert_eq!(stack.base() % kind.body_bytes(), 0);
        }
    }

    #[test]
    fn guard_word_sits_directly_below_the_body_and_survives() {
        for kind in ALL_KINDS {
            let stack = TaskStack::new(kind);
            assert_eq!(
                stack.guard_address(),
                stack.base() - core::mem::size_of::<usize>()
            );
            assert_eq!(stack.guard_word(), STACK_GUARD_MAGIC);
            assert!(stack.guard_intact());
        }
    }

    #[test]
    fn corrupting_the_guard_word_flips_detection() {
        for kind in ALL_KINDS {
            let mut stack = TaskStack::new(kind);
            stack.set_guard_word(!STACK_GUARD_MAGIC);
            assert!(!stack.guard_intact());
        }
    }

    #[test]
    fn body_is_watermark_filled() {
        for kind in ALL_KINDS {
            let stack = TaskStack::new(kind);
            assert_eq!(stack.words().len(), kind.body_words());
            assert!(stack.words().iter().all(|word| *word == STACK_WATERMARK));
        }
    }

    #[test]
    fn top_is_one_past_the_top() {
        for kind in ALL_KINDS {
            let stack = TaskStack::new(kind);
            assert_eq!(stack.top(), stack.base() + stack.size_bytes());
        }
    }

    #[test]
    fn addresses_are_nonzero() {
        for kind in ALL_KINDS {
            let stack = TaskStack::new(kind);
            assert_ne!(stack.base(), 0);
            assert_ne!(stack.top(), 0);
        }
    }

    #[test]
    fn fresh_stack_observes_zero_depth() {
        let mut stack = TaskStack::new(StackKind::Idle);
        assert_eq!(stack.observe_depth(), 0);
        assert!(!stack.is_exhausted());
    }

    #[test]
    fn depth_counts_contiguous_frames_from_the_top() {
        let mut stack = TaskStack::new(StackKind::Idle);
        let top = StackKind::Idle.body_words() - 1;
        stack.set_word(top, 0x1);
        stack.set_word(top - 1, 0x2);
        assert_eq!(stack.observe_depth(), 2);
    }

    #[test]
    fn depth_never_shrinks_once_observed() {
        let mut stack = TaskStack::new(StackKind::Idle);
        let top = StackKind::Idle.body_words() - 1;
        stack.set_word(top, 0x1);
        assert_eq!(stack.observe_depth(), 1);
        stack.set_word(top, STACK_WATERMARK);
        assert_eq!(stack.observe_depth(), 1);
    }

    #[test]
    fn depth_stops_at_watermark_holes() {
        let mut stack = TaskStack::new(StackKind::Idle);
        let top = StackKind::Idle.body_words() - 1;
        stack.set_word(top, 0x1);
        stack.set_word(top - 2, 0x3);
        assert_eq!(stack.observe_depth(), 1);
    }

    #[test]
    fn full_body_is_exhausted() {
        let mut stack = TaskStack::new(StackKind::Idle);
        for index in 0..StackKind::Idle.body_words() {
            stack.set_word(index, index + 1);
        }
        assert_eq!(stack.observe_depth(), StackKind::Idle.body_words());
        assert!(stack.is_exhausted());
    }
}
