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
}
