pub struct SimpleRng {
    state: u32,
}

impl SimpleRng {
    pub fn new(seed: u32) -> Self {
        SimpleRng { state: seed }
    }

    pub fn next(&mut self) -> u32 {
        self.state = self.state.wrapping_mul(1103515245).wrapping_add(12345);
        self.state
    }

    pub fn next_u64(&mut self) -> u64 {
        let hi = self.next() as u64;
        let lo = self.next() as u64;
        (hi << 32) | lo
    }

    pub fn next_range(&mut self, min: u32, max: u32) -> u32 {
        let range = max - min + 1;
        min + (self.next() % range)
    }
}