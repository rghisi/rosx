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

    #[test]
    fn reclaim_block_smaller_than_chunk_returns_it_as_leftover() {
        let mut tracker = ChunkTracker::new(CHUNK_SIZE);
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
