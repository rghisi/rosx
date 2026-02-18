use crate::chunk_allocator::ChunkAllocator;

pub struct ChunkTracker {
    allocator: *mut dyn ChunkAllocator,
    storage: *mut usize,
    chunk_size: usize,
}

impl ChunkTracker {
    pub fn new(allocator: *mut dyn ChunkAllocator) -> Self {
        ChunkTracker {
            allocator,
            storage: core::ptr::null_mut(),
            chunk_size: 0,
        }
    }

    pub fn init(&mut self) {
        let allocator = unsafe { &mut *self.allocator };
        self.chunk_size = allocator.chunk_size();
        let ptr = allocator.allocate_chunks(1).expect("ChunkTracker needs at least one chunk");
        self.storage = ptr as *mut usize;
        unsafe { *self.storage = 0 };
    }

    pub fn chunk_size(&self) -> usize {
        self.chunk_size
    }

    pub fn owned_count(&self) -> usize {
        unsafe { *self.storage }
    }

    pub fn register(&mut self, address: usize) {
        let count = unsafe { *self.storage };
        unsafe { *self.storage.add(count + 1) = address };
        unsafe { *self.storage = count + 1 };
    }

    pub fn reclaim(
        &mut self,
        start: usize,
        size: usize,
        _on_chunk_reclaimed: impl FnMut(usize),
        mut on_leftover: impl FnMut(usize, usize),
    ) {
        on_leftover(start, size);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunk_allocator::ChunkAllocator;

    const CHUNK_SIZE: usize = 4096;

    struct FakeChunkAllocator {
        base: *mut u8,
        size: usize,
        chunk_size: usize,
        allocated: usize,
    }

    impl FakeChunkAllocator {
        fn new(base: *mut u8, size: usize, chunk_size: usize) -> Self {
            FakeChunkAllocator { base, size, chunk_size, allocated: 0 }
        }
    }

    impl ChunkAllocator for FakeChunkAllocator {
        fn chunk_size(&self) -> usize {
            self.chunk_size
        }

        fn allocate_chunks(&mut self, count: usize) -> Option<*mut u8> {
            let needed = count * self.chunk_size;
            if self.allocated + needed > self.size {
                return None;
            }
            let ptr = unsafe { self.base.add(self.allocated) };
            self.allocated += needed;
            Some(ptr)
        }

        fn deallocate_chunks(&mut self, _ptr: *mut u8, _count: usize) {}
    }

    fn create_tracker(allocator: &mut FakeChunkAllocator) -> ChunkTracker {
        let mut tracker = ChunkTracker::new(allocator as *mut dyn ChunkAllocator);
        tracker.init();
        tracker
    }

    #[test]
    fn init_allocates_one_chunk_for_storage() {
        let mut memory = vec![0u8; 4 * CHUNK_SIZE];
        let mut allocator = FakeChunkAllocator::new(memory.as_mut_ptr(), memory.len(), CHUNK_SIZE);

        create_tracker(&mut allocator);

        assert_eq!(allocator.allocated, CHUNK_SIZE);
    }

    #[test]
    fn init_learns_chunk_size_from_allocator() {
        let mut memory = vec![0u8; 4 * CHUNK_SIZE];
        let mut allocator = FakeChunkAllocator::new(memory.as_mut_ptr(), memory.len(), CHUNK_SIZE);

        let tracker = create_tracker(&mut allocator);

        assert_eq!(tracker.chunk_size(), CHUNK_SIZE);
    }

    #[test]
    fn new_tracker_has_zero_owned_chunks() {
        let mut memory = vec![0u8; 4 * CHUNK_SIZE];
        let mut allocator = FakeChunkAllocator::new(memory.as_mut_ptr(), memory.len(), CHUNK_SIZE);

        let tracker = create_tracker(&mut allocator);

        assert_eq!(tracker.owned_count(), 0);
    }

    #[test]
    fn count_is_stored_at_slot_zero() {
        let mut memory = vec![0u8; 4 * CHUNK_SIZE];
        let mut allocator = FakeChunkAllocator::new(memory.as_mut_ptr(), memory.len(), CHUNK_SIZE);

        let mut tracker = create_tracker(&mut allocator);

        let stored_count = unsafe { *tracker.storage };
        assert_eq!(stored_count, 0);

        tracker.register(0xA000);
        let stored_count = unsafe { *tracker.storage };
        assert_eq!(stored_count, 1);

        tracker.register(0xB000);
        let stored_count = unsafe { *tracker.storage };
        assert_eq!(stored_count, 2);
    }

    #[test]
    fn register_stores_addresses_starting_at_slot_one() {
        let mut memory = vec![0u8; 4 * CHUNK_SIZE];
        let mut allocator = FakeChunkAllocator::new(memory.as_mut_ptr(), memory.len(), CHUNK_SIZE);

        let mut tracker = create_tracker(&mut allocator);
        tracker.register(0xA000);
        tracker.register(0xB000);

        let storage = tracker.storage;
        let first = unsafe { *storage.add(1) };
        let second = unsafe { *storage.add(2) };
        assert_eq!(first, 0xA000);
        assert_eq!(second, 0xB000);
    }

    #[test]
    fn register_one_chunk_increments_count() {
        let mut memory = vec![0u8; 4 * CHUNK_SIZE];
        let mut allocator = FakeChunkAllocator::new(memory.as_mut_ptr(), memory.len(), CHUNK_SIZE);

        let mut tracker = create_tracker(&mut allocator);
        tracker.register(0x1000);

        assert_eq!(tracker.owned_count(), 1);
    }

    #[test]
    fn register_two_chunks_increments_count() {
        let mut memory = vec![0u8; 4 * CHUNK_SIZE];
        let mut allocator = FakeChunkAllocator::new(memory.as_mut_ptr(), memory.len(), CHUNK_SIZE);

        let mut tracker = create_tracker(&mut allocator);
        tracker.register(0x1000);
        tracker.register(0x2000);

        assert_eq!(tracker.owned_count(), 2);
    }

    #[test]
    fn reclaim_block_smaller_than_chunk_returns_it_as_leftover() {
        let mut memory = vec![0u8; 4 * CHUNK_SIZE];
        let mut allocator = FakeChunkAllocator::new(memory.as_mut_ptr(), memory.len(), CHUNK_SIZE);

        let mut tracker = create_tracker(&mut allocator);
        tracker.register(0x1000);

        let mut reclaimed = Vec::new();
        let mut leftovers = Vec::new();

        tracker.reclaim(
            0x5000, 512,
            |addr| reclaimed.push(addr),
            |start, size| leftovers.push((start, size)),
        );

        assert!(reclaimed.is_empty());
        assert_eq!(leftovers, vec![(0x5000, 512)]);
        assert_eq!(tracker.owned_count(), 1);
    }
}
