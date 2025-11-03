use kernel::default_output::KernelOutput;
pub struct QemuDebugConsole;

impl QemuDebugConsole {
    const DEBUG_PORT: u16 = 0xe9;

    fn write_byte(&self, byte: u8) {
        unsafe {
            core::arch::asm!(
            "out dx, al",
            in("dx") Self::DEBUG_PORT,
            in("al") byte,
            options(nomem, nostack, preserves_flags)
            );
        }
    }
}

impl KernelOutput for QemuDebugConsole {
    fn write_str(&self, s: &str) {
        for byte in s.bytes() {
            self.write_byte(byte);
        }
    }
}
