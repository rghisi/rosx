pub trait ChunkAllocator {
    fn chunk_size(&self) -> usize;
    fn allocate_chunks(&mut self, count: usize) -> Option<*mut u8>;
    fn deallocate_chunks(&mut self, ptr: *mut u8, count: usize);
}
