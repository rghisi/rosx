use core::arch::asm;
use kernel::cpu::Cpu;

pub struct M68K {}

impl M68K {
    pub const fn new() -> Self {
        M68K {}
    }
}

impl Cpu for M68K {
    fn setup(&self) {
        crate::interrupts::init();
    }

    fn enable_interrupts(&self) {
        // ANDI.W #0xF8FF, SR — clears IPL bits [10:8], accepting all interrupts
        unsafe { asm!(".short 0x027C, 0xF8FF", options(nomem, nostack)) };
    }

    fn disable_interrupts(&self) {
        // ORI.W #0x0700, SR — sets IPL bits [10:8] to 7, masking all maskable interrupts
        unsafe { asm!(".short 0x007C, 0x0700", options(nomem, nostack)) };
    }

    fn are_interrupts_enabled(&self) -> bool {
        let sr: u32;
        // MOVE.W SR, D0 — copies status register to d0
        unsafe { asm!(".short 0x40C0", out("d0") sr, options(nomem, nostack)) };
        (sr >> 8) & 0x7 == 0
    }

    fn initialize_stack(
        &self,
        stack_pointer: usize,
        entry_point: usize,
        param1: usize,
        param2: usize,
    ) -> usize {
        unsafe {
            let mut sp = (stack_pointer & !0x3) as *mut u32;

            // cdecl: rightmost argument first
            sp = sp.sub(1);
            *sp = param2 as u32;
            sp = sp.sub(1);
            *sp = param1 as u32;

            // Sentinel return address — crash visibly if the task function returns
            sp = sp.sub(1);
            *sp = 0;

            // swap_context's rts jumps here, starting the task
            sp = sp.sub(1);
            *sp = entry_point as u32;

            // Initial callee-saved register values that swap_context will pop via MOVEM:
            // D2, D3, D4, D5, D6, D7, A2, A3, A4, A5, A6 (11 registers × 4 bytes)
            for _ in 0..11 {
                sp = sp.sub(1);
                *sp = 0;
            }

            sp as usize
        }
    }

    fn swap_context(&self, stack_pointer_to_store: *mut usize, stack_pointer_to_load: usize) {
        unsafe { swap_context(stack_pointer_to_store, stack_pointer_to_load) };
    }

    fn get_system_time(&self) -> u64 {
        *crate::interrupts::SYSTEM_TIME_MS.borrow() as u64
    }

    fn halt(&self) {
        // STOP #0x2700 — halts the CPU with interrupts disabled
        unsafe { asm!(".short 0x4E72, 0x2700", options(nomem, nostack)) };
    }
}

extern "C" {
    fn swap_context(store: *mut usize, load: usize);
}
