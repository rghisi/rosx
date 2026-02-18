pub struct ChunkTracker {
    chunk_size: usize,
    count: usize,
}

impl ChunkTracker {
    pub fn new(chunk_size: usize) -> Self {
        ChunkTracker {
            chunk_size,
            count: 0,
        }
    }

    pub fn owned_count(&self) -> usize {
        self.count
    }

    pub fn register(&mut self, _address: usize) {
        self.count += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CHUNK_SIZE: usize = 4096;

    #[test]
    fn new_tracker_has_zero_owned_chunks() {
        let tracker = ChunkTracker::new(CHUNK_SIZE);
        assert_eq!(tracker.owned_count(), 0);
    }

    #[test]
    fn register_one_chunk_increments_count() {
        let mut tracker = ChunkTracker::new(CHUNK_SIZE);
        tracker.register(0x1000);
        assert_eq!(tracker.owned_count(), 1);
    }

    #[test]
    fn register_two_chunks_increments_count() {
        let mut tracker = ChunkTracker::new(CHUNK_SIZE);
        tracker.register(0x1000);
        tracker.register(0x2000);
        assert_eq!(tracker.owned_count(), 2);
    }
}
