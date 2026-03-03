pub mod bitmap_chunk_allocator;
pub mod free_list_allocator;
pub mod memory_manager;

pub const MAX_MEMORY_BLOCKS: usize = 32;

#[derive(Copy, Clone)]
pub struct MemoryBlock {
    pub start: usize,
    pub size: usize,
}

#[derive(Copy, Clone)]
pub struct MemoryBlocks {
    pub blocks: [MemoryBlock; MAX_MEMORY_BLOCKS],
    pub count: usize,
}
