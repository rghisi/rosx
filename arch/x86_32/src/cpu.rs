use core::arch::asm;
use core::sync::atomic::Ordering::Relaxed;
use kernel::kernel::kernel;
use kernel::platform::cpu::Cpu;

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
        unsafe { seed_task_frame(stack_pointer, entry_point, param1, param2) }
    }

    fn swap_context(
        &self,
        state: *mut usize,
        stack_pointer_to_store: *mut usize,
        stack_pointer_to_load: usize,
    ) {
        unsafe {
            asm!("/* {0} */", in(reg) state, options(nomem, nostack, preserves_flags));
            swap_context(stack_pointer_to_store, stack_pointer_to_load)
        };
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

/// Writes the initial frame `swap_context` pops; layout in docs/rationale/x86_32-context-switch.md.
unsafe fn seed_task_frame(stack_pointer: usize, entry_point: usize, param1: usize, param2: usize) -> usize {
    unsafe {
        let mut sp = ((stack_pointer & !0xF) - 8) as *mut u32;
        sp = sp.sub(1);
        *sp = param2 as u32;
        sp = sp.sub(1);
        *sp = param1 as u32;
        sp = sp.sub(1);
        *sp = 0;
        sp = sp.sub(1);
        *sp = entry_point as u32;
        for _ in 0..4 {
            sp = sp.sub(1);
            *sp = 0;
        }
        sp as usize
    }
}

/// Rust half of the `int 0x80` path; interrupt-flag discipline in docs/rationale/x86_32-int80-entry.md.
#[unsafe(no_mangle)]
unsafe extern "C" fn syscall_handler(num: usize, arg1: usize, arg2: usize, arg3: usize) -> usize {
    let result = kernel::syscall::handle_syscall(num, arg1, arg2, arg3);
    kernel().maybe_resched();
    result
}

core::arch::global_asm!(
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
    "    ret",                   // jump to next task's saved return address, IF stays 0
);

core::arch::global_asm!(
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
    // The IDT entry is a trap gate, so IF survives entry with the caller's value (1).
    // cli here closes the interrupt window across handle_syscall so a task parking on
    // a future cannot be preempted between blocking and waiter registration — the
    // window SFMASK closes on x86_64. Kernel contexts never re-enable: swap_context
    // leaves IF cleared, so resumed bookkeeping runs masked and interrupts return
    // only with this handler's iretd, which forces IF=1 in the saved eflags (mirrors
    // the or r11,0x200 boundary in x86_64 syscall.S).
    //
    // ebx, ecx, edx are saved/restored so the caller sees them unchanged (mirrors
    // the Linux int 0x80 ABI and matches the usrlib asm declaration).
    ".global int80_handler",
    "int80_handler:",
    "    cli",                    // park-registration window must be IRQ-atomic
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
    "    or dword ptr [esp + 8], 0x200", // force IF=1 in the saved eflags: the only
                                        // kernel→task interrupt boundary on x86_32
    "    iretd",
);
