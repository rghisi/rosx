use kernel::default_output::KernelOutput;
use core::fmt;

pub const GOLDFISH_TTY_BASE: usize = 0xff008000;

pub struct GoldfishTty {
    base: usize,
}

impl GoldfishTty {
    pub const fn new(base: usize) -> Self {
        Self { base }
    }

    pub fn putc(&self, c: u8) {
        let tty_ptr = self.base as *mut u32;
        unsafe {
            core::ptr::write_volatile(tty_ptr, c as u32);
        }
    }
}

impl KernelOutput for GoldfishTty {
    fn write_str(&self, s: &str) {
        for b in s.as_bytes() {
            self.putc(*b);
        }
    }
}

impl fmt::Write for GoldfishTty {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        KernelOutput::write_str(self, s);
        Ok(())
    }
}
