use alloc::boxed::Box;
use alloc::vec;

pub const STACK_WORDS: usize = 2048;

pub struct TaskStack {
    buffer: Box<[usize; STACK_WORDS]>,
}

impl TaskStack {
    #[allow(clippy::new_without_default)]
    pub fn new() -> TaskStack {
        let buffer: Box<[usize; STACK_WORDS]> = vec![0usize; STACK_WORDS]
            .into_boxed_slice()
            .try_into()
            .unwrap_or_else(|_| unreachable!());
        TaskStack { buffer }
    }

    pub fn top(&self) -> usize {
        self.base() + self.size_bytes()
    }

    pub(crate) fn base(&self) -> usize {
        self.buffer.as_ptr().addr()
    }

    pub(crate) fn size_bytes(&self) -> usize {
        STACK_WORDS * core::mem::size_of::<usize>()
    }

    #[cfg(test)]
    pub(crate) fn words(&self) -> &[usize] {
        &self.buffer[..]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn top_is_one_past_the_top() {
        let stack = TaskStack::new();
        assert_eq!(stack.top(), stack.base() + stack.size_bytes());
    }

    #[test]
    fn buffer_is_zero_filled() {
        let stack = TaskStack::new();
        assert!(stack.words().iter().all(|w| *w == 0));
    }

    #[test]
    fn addresses_are_nonzero() {
        let stack = TaskStack::new();
        assert_ne!(stack.base(), 0);
        assert_ne!(stack.top(), 0);
    }
}
