use core::mem::MaybeUninit;

pub struct CircularQueue<T, const N: usize> {
    buffer: [MaybeUninit<T>; N],
    head: usize,
    tail: usize,
    count: usize,
}

impl<T, const N: usize> CircularQueue<T, N> {
    pub fn new() -> Self {
        assert!(N > 0, "CircularQueue capacity must be greater than 0");

        Self {
            buffer: unsafe { MaybeUninit::uninit().assume_init() },
            head: 0,
            tail: 0,
            count: 0,
        }
    }

    pub fn push(&mut self, value: T) -> Result<(), T> {
        if self.is_full() {
            return Err(value);
        }

        unsafe {
            self.buffer[self.tail].as_mut_ptr().write(value);
        }

        self.tail += 1;
        if self.tail >= N {
            self.tail = 0;
        }
        self.count += 1;

        Ok(())
    }

    pub fn pop(&mut self) -> Option<T> {
        if self.is_empty() {
            return None;
        }

        let value = unsafe {
            self.buffer[self.head].as_ptr().read()
        };

        self.head += 1;
        if self.head >= N {
            self.head = 0;
        }
        self.count -= 1;

        Some(value)
    }

    pub fn peek(&self) -> Option<&T> {
        if self.is_empty() {
            return None;
        }

        unsafe {
            Some(&*self.buffer[self.head].as_ptr())
        }
    }

    pub fn peek_mut(&mut self) -> Option<&mut T> {
        if self.is_empty() {
            return None;
        }

        unsafe {
            Some(&mut *self.buffer[self.head].as_mut_ptr())
        }
    }

    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    pub fn is_full(&self) -> bool {
        self.count == N
    }

    pub fn len(&self) -> usize {
        self.count
    }

    pub fn capacity(&self) -> usize {
        N
    }

    pub fn clear(&mut self) {
        while let Some(_) = self.pop() {}
    }
}

impl<T, const N: usize> Drop for CircularQueue<T, N> {
    fn drop(&mut self) {
        self.clear();
    }
}

#[cfg(test)]
mod tests {
    use core::cmp::Ordering;
    use core::sync::Arc;
    use core::sync::atomic::AtomicUsize;
    use super::*;

    #[test]
    fn test_new() {
        let queue: CircularQueue<u32, 4> = CircularQueue::new();
        assert_eq!(queue.len(), 0);
        assert_eq!(queue.capacity(), 4);
        assert!(queue.is_empty());
        assert!(!queue.is_full());
    }

    #[test]
    #[should_panic(expected = "CircularQueue capacity must be greater than 0")]
    fn test_zero_capacity_panics() {
        let _queue: CircularQueue<u32, 0> = CircularQueue::new();
    }

    #[test]
    fn test_push_pop() {
        let mut queue: CircularQueue<u32, 3> = CircularQueue::new();

        assert!(queue.push(1).is_ok());
        assert!(queue.push(2).is_ok());
        assert!(queue.push(3).is_ok());

        assert_eq!(queue.len(), 3);
        assert!(queue.is_full());

        assert_eq!(queue.pop(), Some(1));
        assert_eq!(queue.pop(), Some(2));
        assert_eq!(queue.pop(), Some(3));
        assert_eq!(queue.pop(), None);

        assert!(queue.is_empty());
    }

    #[test]
    fn test_push_when_full() {
        let mut queue: CircularQueue<u32, 2> = CircularQueue::new();

        queue.push(1).unwrap();
        queue.push(2).unwrap();

        assert!(queue.is_full());
        assert_eq!(queue.push(3), Err(3));
        assert_eq!(queue.len(), 2);
    }

    #[test]
    fn test_wraparound() {
        let mut queue: CircularQueue<u32, 3> = CircularQueue::new();

        queue.push(1).unwrap();
        queue.push(2).unwrap();
        queue.push(3).unwrap();

        assert_eq!(queue.pop(), Some(1));
        assert_eq!(queue.pop(), Some(2));

        queue.push(4).unwrap();
        queue.push(5).unwrap();

        assert_eq!(queue.pop(), Some(3));
        assert_eq!(queue.pop(), Some(4));
        assert_eq!(queue.pop(), Some(5));
        assert_eq!(queue.pop(), None);
    }

    #[test]
    fn test_peek() {
        let mut queue: CircularQueue<u32, 3> = CircularQueue::new();

        assert_eq!(queue.peek(), None);

        queue.push(1).unwrap();
        queue.push(2).unwrap();

        assert_eq!(queue.peek(), Some(&1));
        assert_eq!(queue.len(), 2);

        assert_eq!(queue.pop(), Some(1));
        assert_eq!(queue.peek(), Some(&2));
    }

    #[test]
    fn test_peek_mut() {
        let mut queue: CircularQueue<u32, 3> = CircularQueue::new();

        queue.push(1).unwrap();

        if let Some(val) = queue.peek_mut() {
            *val = 42;
        }

        assert_eq!(queue.pop(), Some(42));
    }

    #[test]
    fn test_clear() {
        let mut queue: CircularQueue<u32, 3> = CircularQueue::new();

        queue.push(1).unwrap();
        queue.push(2).unwrap();
        queue.push(3).unwrap();

        assert_eq!(queue.len(), 3);

        queue.clear();

        assert_eq!(queue.len(), 0);
        assert!(queue.is_empty());
    }

    #[test]
    fn test_drop_elements() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;

        #[derive(Debug)]
        struct DropCounter {
            counter: Arc<AtomicUsize>,
        }

        impl Drop for DropCounter {
            fn drop(&mut self) {
                self.counter.fetch_add(1, Ordering::SeqCst);
            }
        }

        let drop_count = Arc::new(AtomicUsize::new(0));

        {
            let mut queue: CircularQueue<DropCounter, 3> = CircularQueue::new();

            queue.push(DropCounter { counter: drop_count.clone() }).unwrap();
            queue.push(DropCounter { counter: drop_count.clone() }).unwrap();

            assert_eq!(drop_count.load(Ordering::SeqCst), 0);

            queue.pop();
            assert_eq!(drop_count.load(Ordering::SeqCst), 1);
        }

        assert_eq!(drop_count.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn test_with_string() {
        let mut queue: CircularQueue<String, 3> = CircularQueue::new();

        queue.push("hello".to_string()).unwrap();
        queue.push("world".to_string()).unwrap();

        assert_eq!(queue.pop(), Some("hello".to_string()));
        assert_eq!(queue.peek(), Some(&"world".to_string()));
    }
}