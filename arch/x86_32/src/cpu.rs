use core::arch::asm;
use core::sync::atomic::Ordering::Relaxed;
use kernel::cpu::Cpu;

pub struct X86_32 {}

impl X86_32 {
    pub const fn new() -> Self {
        X86_32 {}
    }
}

impl Cpu for X86_32 {
    fn setup(&self) {
        crate::interrupts::init();
        crate::interrupts::enable_timer();
        crate::interrupts::enable_keyboard();
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
        crate::interrupts::SYSTEM_TIME_MS.load(Relaxed) as u64
    }

    fn halt(&self) {
        unsafe { asm!("hlt", options(nomem, nostack, preserves_flags)) };
    }
}

unsafe extern "C" {
    fn swap_context(store: *mut usize, load: usize);
}

#[unsafe(no_mangle)]
unsafe extern "C" fn syscall_handler(num: usize, arg1: usize, arg2: usize, arg3: usize) -> usize {
    kernel::syscall::handle_syscall(num, arg1, arg2, arg3)
}

// swap_context(store: *mut usize, load: usize)
//
// Stack on entry (after push of return address by call instruction):
//   [esp+4]  = store — address where current esp should be saved (may be null)
//   [esp+8]  = load  — esp value to restore for the next task
//
// Arguments are read before any push so that offsets stay fixed at +4/+8.
// When store is null the current context is abandoned entirely (no save).
//
// A suspended task's stack holds (from lowest address):
//   [sp+0]   edi
//   [sp+4]   esi
//   [sp+8]   ebx
//   [sp+12]  ebp
//   [sp+16]  return address / task entry point (popped by ret)
core::arch::global_asm!(
    ".global swap_context",
    "swap_context:",
    "    cli",
    "    mov eax, [esp + 4]",   // store ptr — read BEFORE any pushes
    "    mov ecx, [esp + 8]",   // load value — read BEFORE any pushes
    "    test eax, eax",
    "    jz 1f",                 // if null: abandon current context, skip save
    "    push ebp",
    "    push ebx",
    "    push esi",
    "    push edi",
    "    mov [eax], esp",        // *store = esp (points to complete register frame)
    "1:",
    "    mov esp, ecx",          // switch to next task's stack
    "    pop edi",
    "    pop esi",
    "    pop ebx",
    "    pop ebp",
    "    sti",
    "    ret",                   // jump to next task's saved return address
);

// int80_handler — raw entry point for `int 0x80` system calls.
//
// Calling convention (matching usrlib/src/arch/x86_32.rs):
//   eax = syscall number
//   ebx = arg1
//   ecx = arg2
//   edx = arg3
//   return value: eax
//
// The CPU pushes [eip, cs, eflags] before entering. iretd restores them on exit,
// which means eflags is preserved from the caller's perspective (preserves_flags).
//
// ebx, ecx, edx are saved/restored so the caller sees them unchanged (mirrors
// the Linux int 0x80 ABI and matches the usrlib asm declaration).
core::arch::global_asm!(
    ".global int80_handler",
    "int80_handler:",
    "    push ebp",
    "    push edi",
    "    push esi",
    "    push edx",             // save arg3 (caller-saved in cdecl, but we preserve it)
    "    push ecx",             // save arg2
    "    push ebx",             // save arg1
    "    push edx",             // cdecl arg3
    "    push ecx",             // cdecl arg2
    "    push ebx",             // cdecl arg1
    "    push eax",             // cdecl num
    "    call syscall_handler", // result in eax
    "    add esp, 16",          // discard 4 cdecl args
    "    pop ebx",
    "    pop ecx",
    "    pop edx",
    "    pop esi",
    "    pop edi",
    "    pop ebp",
    "    iretd",
);
