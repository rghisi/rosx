#[derive(Copy, Clone)]
pub struct MemoryRegion {
    pub start: usize,
    pub size: usize,
}

impl MemoryRegion {
    pub fn new(start: usize, size: usize) -> Self {
        MemoryRegion { start, size }
    }
}
