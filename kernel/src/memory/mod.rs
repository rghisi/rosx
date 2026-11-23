pub mod memory_manager;
pub mod bitmap_chunk_allocator;
pub mod free_list_allocator;
pub mod program_memory;

pub const MAX_MEMORY_BLOCKS: usize = 32;

#[derive(Copy, Clone, PartialEq, Eq)]
pub enum BlockUse {
    Free,
    Held,
}

#[derive(Copy, Clone)]
pub struct MemoryBlock {
    pub start: usize,
    pub size: usize,
    pub use_: BlockUse,
}

impl MemoryBlock {
    pub const fn free(start: usize, size: usize) -> Self {
        MemoryBlock {
            start,
            size,
            use_: BlockUse::Free,
        }
    }
}

#[derive(Copy, Clone)]
pub struct MemoryBlocks {
    pub blocks: [MemoryBlock; MAX_MEMORY_BLOCKS],
    pub count: usize,
}

impl MemoryBlocks {
    pub fn free_blocks(&self) -> impl DoubleEndedIterator<Item = &MemoryBlock> + '_ {
        self.blocks[..self.count]
            .iter()
            .filter(|block| block.use_ == BlockUse::Free)
    }
}
