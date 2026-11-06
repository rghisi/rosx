pub mod memory_manager;
pub mod bitmap_chunk_allocator;
pub mod free_list_allocator;

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

impl MemoryBlocks {
    pub fn carve(&mut self, start: usize, end: usize) {
        if start >= end {
            return;
        }
        let mut carved: [MemoryBlock; MAX_MEMORY_BLOCKS] =
            core::array::from_fn(|_| MemoryBlock { start: 0, size: 0 });
        let mut carved_count = 0usize;
        for block in self.blocks[..self.count].iter() {
            let block_start = block.start;
            let block_end = block.start + block.size;
            if block_end <= start || block_start >= end {
                carved[carved_count] = *block;
                carved_count += 1;
                continue;
            }
            if block_start < start {
                if carved_count == MAX_MEMORY_BLOCKS {
                    panic!("[MEMORY] block capacity exhausted by carve split");
                }
                carved[carved_count] = MemoryBlock { start: block_start, size: start - block_start };
                carved_count += 1;
            }
            if end < block_end {
                if carved_count == MAX_MEMORY_BLOCKS {
                    panic!("[MEMORY] block capacity exhausted by carve split");
                }
                carved[carved_count] = MemoryBlock { start: end, size: block_end - end };
                carved_count += 1;
            }
        }
        self.blocks = carved;
        self.count = carved_count;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blocks(entries: &[(usize, usize)]) -> MemoryBlocks {
        let mut out = MemoryBlocks {
            blocks: core::array::from_fn(|_| MemoryBlock { start: 0, size: 0 }),
            count: 0,
        };
        for &(start, size) in entries {
            out.blocks[out.count] = MemoryBlock { start, size };
            out.count += 1;
        }
        out
    }

    fn pairs(memory: &MemoryBlocks) -> Vec<(usize, usize)> {
        memory.blocks[..memory.count]
            .iter()
            .map(|b| (b.start, b.size))
            .collect()
    }

    #[test]
    fn carve_without_overlap_keeps_every_block() {
        let mut memory = blocks(&[(0x1000, 0x1000), (0x8000, 0x2000)]);
        memory.carve(0x4000, 0x5000);
        assert_eq!(pairs(&memory), vec![(0x1000, 0x1000), (0x8000, 0x2000)]);
    }

    #[test]
    fn carve_inside_a_block_splits_it_in_two() {
        let mut memory = blocks(&[(0x1000, 0x4000)]);
        memory.carve(0x2000, 0x3000);
        assert_eq!(pairs(&memory), vec![(0x1000, 0x1000), (0x3000, 0x2000)]);
    }

    #[test]
    fn carve_of_a_prefix_shrinks_the_block() {
        let mut memory = blocks(&[(0x1000, 0x4000)]);
        memory.carve(0x1000, 0x2000);
        assert_eq!(pairs(&memory), vec![(0x2000, 0x3000)]);
    }

    #[test]
    fn carve_of_a_suffix_shrinks_the_block() {
        let mut memory = blocks(&[(0x1000, 0x4000)]);
        memory.carve(0x4000, 0x5000);
        assert_eq!(pairs(&memory), vec![(0x1000, 0x3000)]);
    }

    #[test]
    fn carve_of_a_whole_block_drops_it() {
        let mut memory = blocks(&[(0x1000, 0x1000), (0x4000, 0x1000)]);
        memory.carve(0x1000, 0x2000);
        assert_eq!(pairs(&memory), vec![(0x4000, 0x1000)]);
    }

    #[test]
    fn carve_spanning_several_blocks_keeps_only_outer_fragments() {
        let mut memory = blocks(&[(0x1000, 0x2000), (0x4000, 0x2000), (0x8000, 0x2000)]);
        memory.carve(0x2000, 0x9000);
        assert_eq!(pairs(&memory), vec![(0x1000, 0x1000), (0x9000, 0x1000)]);
    }

    #[test]
    fn touching_edges_keep_the_block_whole() {
        let mut memory = blocks(&[(0x1000, 0x1000), (0x4000, 0x1000)]);
        memory.carve(0x2000, 0x4000);
        assert_eq!(pairs(&memory), vec![(0x1000, 0x1000), (0x4000, 0x1000)]);
    }

    #[test]
    fn empty_carve_range_is_a_noop() {
        let mut memory = blocks(&[(0x1000, 0x1000)]);
        memory.carve(0x2000, 0x2000);
        memory.carve(0x3000, 0x2000);
        assert_eq!(pairs(&memory), vec![(0x1000, 0x1000)]);
    }
}