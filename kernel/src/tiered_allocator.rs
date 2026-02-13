pub trait BlockProvider {
    fn block_size(&self) -> usize;
    fn allocate_blocks(&self, min_blocks: usize) -> Option<(*mut u8, usize)>;
    fn release_blocks(&self, ptr: *mut u8, count: usize);
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    use std::cell::RefCell;
    use std::vec;
    use std::vec::Vec;

    struct MockBlockProvider {
        block_size: usize,
        memory: Vec<u8>,
        next_offset: RefCell<usize>,
        released: RefCell<Vec<(*mut u8, usize)>>,
    }

    impl MockBlockProvider {
        fn new(block_size: usize, total_blocks: usize) -> Self {
            MockBlockProvider {
                block_size,
                memory: vec![0u8; block_size * total_blocks],
                next_offset: RefCell::new(0),
                released: RefCell::new(Vec::new()),
            }
        }

        fn released_count(&self) -> usize {
            self.released.borrow().len()
        }
    }

    impl BlockProvider for MockBlockProvider {
        fn block_size(&self) -> usize {
            self.block_size
        }

        fn allocate_blocks(&self, min_blocks: usize) -> Option<(*mut u8, usize)> {
            let mut offset = self.next_offset.borrow_mut();
            let bytes_needed = min_blocks * self.block_size;
            if *offset + bytes_needed > self.memory.len() {
                return None;
            }
            let ptr = unsafe { self.memory.as_ptr().add(*offset) as *mut u8 };
            *offset += bytes_needed;
            Some((ptr, min_blocks))
        }

        fn release_blocks(&self, ptr: *mut u8, count: usize) {
            self.released.borrow_mut().push((ptr, count));
        }
    }

    #[test]
    fn mock_provider_should_allocate_blocks() {
        let provider = MockBlockProvider::new(4096, 4);
        let result = provider.allocate_blocks(1);
        assert!(result.is_some());
        let (ptr, count) = result.unwrap();
        assert!(!ptr.is_null());
        assert_eq!(count, 1);
    }

    #[test]
    fn mock_provider_should_allocate_multiple_blocks() {
        let provider = MockBlockProvider::new(4096, 4);
        let (ptr1, _) = provider.allocate_blocks(2).unwrap();
        let (ptr2, _) = provider.allocate_blocks(2).unwrap();
        assert_eq!(unsafe { ptr2.offset_from(ptr1) }, 8192);
    }

    #[test]
    fn mock_provider_should_return_none_when_exhausted() {
        let provider = MockBlockProvider::new(4096, 2);
        provider.allocate_blocks(2).unwrap();
        assert!(provider.allocate_blocks(1).is_none());
    }

    #[test]
    fn mock_provider_should_track_releases() {
        let provider = MockBlockProvider::new(4096, 4);
        let (ptr, _) = provider.allocate_blocks(1).unwrap();
        provider.release_blocks(ptr, 1);
        assert_eq!(provider.released_count(), 1);
    }
}
