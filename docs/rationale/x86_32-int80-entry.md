# x86_32 `int 0x80` entry and interrupt-flag discipline

## Why this is hard
User tasks enter the kernel through a trap gate, so IF survives entry with the caller's value (1). Between a task registering as a waiter and actually parking, a timer interrupt could preempt it; the entry path must close that window without trapping kernel contexts in permanent IF=0.

## Invariants relied on
- Calling convention (matches `usrlib/src/arch/x86_32.rs`): `eax` = syscall number, `ebx`/`ecx`/`edx` = arguments 1 to 3, result in `eax`. `ebx`, `ecx`, `edx` are saved and restored so the caller sees them unchanged (the Linux `int 0x80` ABI).
- The CPU pushes `eip`, `cs`, `eflags`; `iretd` restores them, so flags are preserved from the caller's view.
- `cli` first closes the interrupt window across `handle_syscall`; this is the window SFMASK closes on x86_64.
- Kernel contexts never re-enable interrupts: `swap_context` leaves IF clear, so resumed bookkeeping runs masked. The only kernel-to-task interrupt boundary is this handler's `iretd`, which forces IF=1 in the saved eflags (`or dword ptr [esp + 8], 0x200`), mirroring the `or r11, 0x200` boundary in the x86_64 `syscall.S`.
- `syscall_handler` ends with `maybe_resched()`, the single forced-context-switch site at syscall exit.

## What breaks if changed
Dropping the `cli` lets a task be preempted between blocking and waiter registration (lost wakeups). Dropping the forced IF=1 leaves tasks running with interrupts masked forever after the first resume.

## Evidence
`plans/x86_32-wake-race.md`, `plans/stack/deferred-reschedule-impl.md`, and the `kstats_mem` integration test on both arches.

## Status
verified in QEMU; unverified on hardware.
