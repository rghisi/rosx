use kernel::default_output::KernelOutput;

pub struct QemuDebugConsole;

impl KernelOutput for QemuDebugConsole {
    fn write_str(&self, s: &str) {
        for byte in s.bytes() {
            unsafe {
                core::arch::asm!(
                    "out dx, al",
                    in("dx") 0xE9u16,
                    in("al") byte,
                    options(nomem, nostack, preserves_flags)
                );
            }
        }
    }
}
