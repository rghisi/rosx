use kernel::default_output::KernelOutput;
use volatile::Volatile;
use core::fmt;

pub const GOLDFISH_TTY_BASE: usize = 0xff008000;

#[repr(C)]
struct GoldfishTtyRegisters {
    put_char: Volatile<u32>,
}

pub struct GoldfishTty {
    base: usize,
}

impl GoldfishTty {
    pub const fn new(base: usize) -> Self {
        Self { base }
    }

    fn registers(&self) -> &mut GoldfishTtyRegisters {
        unsafe { &mut *(self.base as *mut GoldfishTtyRegisters) }
    }

    pub fn putc(&self, c: u8) {
        self.registers().put_char.write(c as u32);
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
