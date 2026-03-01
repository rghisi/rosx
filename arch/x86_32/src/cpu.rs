use core::arch::asm;
use kernel::cpu::Cpu;

pub struct X86_32 {}

impl X86_32 {
    pub const fn new() -> Self {
        X86_32 {}
    }
}

impl Cpu for X86_32 {
    fn setup(&self) {
        // Interrupt initialisation added in next step
    }

    fn enable_interrupts(&self) {
        unsafe { asm!("sti", options(nomem, nostack)) };
    }

    fn disable_interrupts(&self) {
        unsafe { asm!("cli", options(nomem, nostack)) };
    }

    fn are_interrupts_enabled(&self) -> bool {
        let flags: u32;
        unsafe {
            asm!(
                "pushfd",
                "pop {flags}",
                flags = out(reg) flags,
            );
        }
        flags & 0x200 != 0
    }

    fn initialize_stack(
        &self,
        stack_pointer: usize,
        entry_point: usize,
        param1: usize,
        param2: usize,
    ) -> usize {
        unsafe {
            // Align to 16, then subtract 8 so that after swap_context's ret
            // esp satisfies the System V i386 ABI (esp ≡ 12 mod 16 at task entry).
            let mut sp = ((stack_pointer & !0xF) - 8) as *mut u32;

            // cdecl arguments (rightmost first, so param1 ends up at [esp+4]).
            sp = sp.sub(1);
            *sp = param2 as u32;
            sp = sp.sub(1);
            *sp = param1 as u32;

            // Sentinel return address — if the task function ever returns it crashes
            // visibly rather than executing garbage.
            sp = sp.sub(1);
            *sp = 0;

            // swap_context's ret jumps here, starting the task.
            sp = sp.sub(1);
            *sp = entry_point as u32;

            // Initial callee-saved register values that swap_context will pop.
            sp = sp.sub(1);
            *sp = 0; // ebp
            sp = sp.sub(1);
            *sp = 0; // ebx
            sp = sp.sub(1);
            *sp = 0; // esi
            sp = sp.sub(1);
            *sp = 0; // edi  ← new task's initial esp

            sp as usize
        }
    }

    fn swap_context(&self, stack_pointer_to_store: *mut usize, stack_pointer_to_load: usize) {
        unsafe { swap_context(stack_pointer_to_store, stack_pointer_to_load) };
    }

    fn get_system_time(&self) -> u64 {
        0
    }

    fn halt(&self) {
        unsafe { asm!("hlt", options(nomem, nostack, preserves_flags)) };
    }
}

unsafe extern "C" {
    fn swap_context(store: *mut usize, load: usize);
}

// swap_context(store: *mut usize, load: usize)
//
// Stack on entry (after push of return address by call instruction):
//   [esp+4]  = store — address where current esp should be saved (may be null)
//   [esp+8]  = load  — esp value to restore for the next task
//
// Callee-saved registers are pushed so that a suspended task's stack holds:
//   [sp+0]   edi
//   [sp+4]   esi
//   [sp+8]   ebx
//   [sp+12]  ebp
//   [sp+16]  return address / task entry point (popped by ret)
core::arch::global_asm!(
    ".global swap_context",
    "swap_context:",
    "    push ebp",
    "    push ebx",
    "    push esi",
    "    push edi",
    "    mov eax, [esp + 20]",  // store ptr (offset past 4 pushes + return addr)
    "    mov ecx, [esp + 24]",  // load value
    "    test eax, eax",
    "    jz 1f",
    "    mov [eax], esp",        // *store = current esp
    "1:",
    "    mov esp, ecx",          // switch to next task's stack
    "    pop edi",
    "    pop esi",
    "    pop ebx",
    "    pop ebp",
    "    ret",                   // jump to next task's saved return address
);
