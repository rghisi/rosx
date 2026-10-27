pub struct SimpleRng {
    state: u32,
}

impl SimpleRng {
    pub fn new(seed: u32) -> Self {
        SimpleRng {
            state: if seed == 0 { 0x6D2B_79F5 } else { seed },
        }
    }

    pub fn next(&mut self) -> u32 {
        let mut value = self.state;
        value ^= value << 13;
        value ^= value >> 17;
        value ^= value << 5;
        self.state = value;
        value
    }

    pub fn next_u64(&mut self) -> u64 {
        let high = self.next() as u64;
        let low = self.next() as u64;
        (high << 32) | low
    }

    pub fn next_range(&mut self, min: u32, max: u32) -> u32 {
        let span = (max - min) as u64 + 1;
        min + (((self.next() as u64 * span) >> 32) as u32)
    }

    pub fn coin(&mut self) -> bool {
        self.next() >> 31 != 0
    }
}
