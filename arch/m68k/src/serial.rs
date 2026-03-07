use kernel::default_output::KernelOutput;

const GOLDFISH_TTY_BASE: usize = 0xff008000;

pub struct GoldfishSerial;

impl GoldfishSerial {
    pub const fn new() -> Self {
        GoldfishSerial
    }
}

impl KernelOutput for GoldfishSerial {
    fn write_str(&self, s: &str) {
        let reg = GOLDFISH_TTY_BASE as *mut u32;
        for byte in s.bytes() {
            // SAFETY: Goldfish TTY MMIO address is fixed in the QEMU virt machine
            unsafe { core::ptr::write_volatile(reg, byte as u32) };
        }
    }
}
