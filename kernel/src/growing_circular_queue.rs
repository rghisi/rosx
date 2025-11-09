use alloc::alloc::Layout;
use alloc::alloc::alloc;
use alloc::boxed::Box;
use core::cmp::min;
use core::mem::MaybeUninit;
use core::ptr;
use core::slice;

pub struct GrowingQueue<T> {
    buffer: Box<[MaybeUninit<T>]>,
    head: usize,
    tail: usize,
    count: usize,
    max_capacity: usize,
}

impl<T> GrowingQueue<T> {
    pub fn new(initial_capacity: usize, max_capacity: Option<usize>) -> Self {
        assert!(
            initial_capacity > 0,
            "GrowingQueue initial capacity must be greater than 0"
        );

        if let Some(max) = max_capacity {
            assert!(
                initial_capacity <= max,
                "GrowingQueue initial capacity must not exceed max capacity"
            );
        }

        let max = if max_capacity.is_some() {
            max_capacity.unwrap()
        } else {
            usize::MAX
        };

        let buffer = Self::allocate_buffer(initial_capacity);

        Self {
            buffer,
            head: 0,
            tail: 0,
            count: 0,
            max_capacity: max,
        }
    }

    fn allocate_buffer(capacity: usize) -> Box<[MaybeUninit<T>]> {
        unsafe {
            let layout = Layout::array::<MaybeUninit<T>>(capacity).unwrap();
            let ptr = alloc(layout) as *mut MaybeUninit<T>;
            if ptr.is_null() {
                alloc::alloc::handle_alloc_error(layout);
            }
            let slice = slice::from_raw_parts_mut(ptr, capacity);
            Box::from_raw(slice)
        }
    }

    fn grow(&mut self) -> Result<(), ()> {
        let old_capacity = self.buffer.len();
        let new_capacity = min(old_capacity + old_capacity, self.max_capacity);

        if new_capacity != old_capacity {
            let mut new_buffer = Self::allocate_buffer(new_capacity);
            let mut src_idx = self.head;
            for dst_idx in 0..self.count {
                unsafe {
                    let src_ptr = self.buffer[src_idx].as_ptr();
                    let dst_ptr = new_buffer[dst_idx].as_mut_ptr();
                    ptr::copy_nonoverlapping(src_ptr, dst_ptr, 1);
                }

                src_idx += 1;
                if src_idx >= old_capacity {
                    src_idx = 0;
                }
            }

            self.buffer = new_buffer;
            self.head = 0;
            self.tail = self.count;
        }

        Ok(())
    }

    pub fn push(&mut self, value: T) -> Result<(), T> {
        if self.is_full() {
            if self.grow().is_err() {
                return Err(value);
            }
        }

        if self.is_full() {
            return Err(value);
        }

        unsafe {
            self.buffer[self.tail].as_mut_ptr().write(value);
        }

        self.tail += 1;
        if self.tail >= self.buffer.len() {
            self.tail = 0;
        }
        self.count += 1;

        Ok(())
    }

    pub fn pop(&mut self) -> Option<T> {
        if self.is_empty() {
            return None;
        }

        let value = unsafe { self.buffer[self.head].as_ptr().read() };

        self.head += 1;
        if self.head >= self.buffer.len() {
            self.head = 0;
        }
        self.count -= 1;

        Some(value)
    }

    pub fn peek(&self) -> Option<&T> {
        if self.is_empty() {
            return None;
        }

        unsafe { Some(&*self.buffer[self.head].as_ptr()) }
    }

    pub fn peek_mut(&mut self) -> Option<&mut T> {
        if self.is_empty() {
            return None;
        }

        unsafe { Some(&mut *self.buffer[self.head].as_mut_ptr()) }
    }

    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    fn is_full(&self) -> bool {
        self.count == self.buffer.len()
    }

    pub fn len(&self) -> usize {
        self.count
    }

    pub fn capacity(&self) -> usize {
        self.buffer.len()
    }

    pub fn clear(&mut self) {
        while let Some(_) = self.pop() {}
    }

    fn can_grow(&self) -> bool {
        self.capacity() < self.max_capacity
    }
}

impl<T> Drop for GrowingQueue<T> {
    fn drop(&mut self) {
        self.clear();
    }
}

#[cfg(test)]
mod tests {
    extern crate std as core;

    use crate::growing_circular_queue::GrowingQueue;
    use alloc::string::{String, ToString};
    use core::sync::Arc;
    use core::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn test_new() {
        let queue: GrowingQueue<u32> = GrowingQueue::new(4, None);
        assert_eq!(queue.len(), 0);
        assert_eq!(queue.capacity(), 4);
        assert!(queue.is_empty());
    }

    #[test]
    #[should_panic(expected = "GrowingQueue initial capacity must be greater than 0")]
    fn test_zero_capacity_panics() {
        let _queue: GrowingQueue<u32> = GrowingQueue::new(0, None);
    }

    #[test]
    #[should_panic(expected = "GrowingQueue initial capacity must not exceed max capacity")]
    fn test_initial_exceeds_max_panics() {
        let _queue: GrowingQueue<u32> = GrowingQueue::new(10, Some(5));
    }

    #[test]
    fn test_push_pop() {
        let mut queue: GrowingQueue<u32> = GrowingQueue::new(3, None);

        assert!(queue.push(1).is_ok());
        assert!(queue.push(2).is_ok());
        assert!(queue.push(3).is_ok());

        assert_eq!(queue.len(), 3);

        assert_eq!(queue.pop(), Some(1));
        assert_eq!(queue.pop(), Some(2));
        assert_eq!(queue.pop(), Some(3));
        assert_eq!(queue.pop(), None);

        assert!(queue.is_empty());
    }

    #[test]
    fn test_automatic_growth() {
        let mut queue: GrowingQueue<u32> = GrowingQueue::new(2, None);

        assert_eq!(queue.capacity(), 2);

        queue.push(1).unwrap();
        queue.push(2).unwrap();

        assert_eq!(queue.capacity(), 2);

        queue.push(3).unwrap();

        assert_eq!(queue.capacity(), 4);
        assert_eq!(queue.len(), 3);

        assert_eq!(queue.pop(), Some(1));
        assert_eq!(queue.pop(), Some(2));
        assert_eq!(queue.pop(), Some(3));
    }

    #[test]
    fn test_growth_preserves_order() {
        let mut queue: GrowingQueue<u32> = GrowingQueue::new(2, None);

        queue.push(1).unwrap();
        queue.push(2).unwrap();
        queue.pop();
        queue.push(3).unwrap();
        queue.push(4).unwrap();

        assert_eq!(queue.capacity(), 4);

        assert_eq!(queue.pop(), Some(2));
        assert_eq!(queue.pop(), Some(3));
        assert_eq!(queue.pop(), Some(4));
    }

    #[test]
    fn test_max_capacity_limit() {
        let mut queue: GrowingQueue<u32> = GrowingQueue::new(2, Some(2));

        queue.push(1).unwrap();
        queue.push(2).unwrap();

        assert_eq!(queue.push(3), Err(3));
        assert_eq!(queue.len(), 2);
        assert_eq!(queue.capacity(), 2);
    }

    #[test]
    fn test_growth_stops_at_max() {
        let mut queue: GrowingQueue<u32> = GrowingQueue::new(2, Some(3));

        queue.push(1).unwrap();
        queue.push(2).unwrap();
        queue.push(3).unwrap();

        assert_eq!(queue.capacity(), 3);
        assert_eq!(queue.push(4), Err(4));
    }

    #[test]
    fn test_multiple_growths() {
        let mut queue: GrowingQueue<u32> = GrowingQueue::new(2, None);

        for i in 0..10 {
            queue.push(i).unwrap();
        }

        assert_eq!(queue.capacity(), 16);
        assert_eq!(queue.len(), 10);

        for i in 0..10 {
            assert_eq!(queue.pop(), Some(i));
        }
    }

    #[test]
    fn test_peek() {
        let mut queue: GrowingQueue<u32> = GrowingQueue::new(3, None);

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
        let mut queue: GrowingQueue<u32> = GrowingQueue::new(3, None);

        queue.push(1).unwrap();

        if let Some(val) = queue.peek_mut() {
            *val = 42;
        }

        assert_eq!(queue.pop(), Some(42));
    }

    #[test]
    fn test_clear() {
        let mut queue: GrowingQueue<u32> = GrowingQueue::new(3, None);

        queue.push(1).unwrap();
        queue.push(2).unwrap();
        queue.push(3).unwrap();

        assert_eq!(queue.len(), 3);

        queue.clear();

        assert_eq!(queue.len(), 0);
        assert!(queue.is_empty());
    }

    #[test]
    fn test_with_string() {
        let mut queue: GrowingQueue<String> = GrowingQueue::new(2, None);

        queue.push("hello".to_string()).unwrap();
        queue.push("world".to_string()).unwrap();
        queue.push("test".to_string()).unwrap();

        assert_eq!(queue.capacity(), 4);
        assert_eq!(queue.pop(), Some("hello".to_string()));
        assert_eq!(queue.peek(), Some(&"world".to_string()));
    }

    #[test]
    fn test_drop_elements() {
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
            let mut queue: GrowingQueue<DropCounter> = GrowingQueue::new(2, None);

            queue
                .push(DropCounter {
                    counter: drop_count.clone(),
                })
                .unwrap();
            queue
                .push(DropCounter {
                    counter: drop_count.clone(),
                })
                .unwrap();

            assert_eq!(drop_count.load(Ordering::SeqCst), 0);

            queue.pop();
            assert_eq!(drop_count.load(Ordering::SeqCst), 1);
        }

        assert_eq!(drop_count.load(Ordering::SeqCst), 2);
    }
}
