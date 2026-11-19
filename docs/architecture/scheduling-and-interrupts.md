# Scheduling and interrupts

## Invariants

- Kernel code runs with IF=0 except idle `hlt` and the user-return boundaries; scheduler, future-registry and mailbox mutations are `IrqGuard` sections, and the `[IRQGUARD]` re-entry tripwire must stay silent.
- Level-triggered event IRQ lines stay masked after dispatch until the driver calls `IrqClear` after clearing its device's interrupt source; only edge registrations (timer, keyboard) auto-unmask.
- `Kernel::maybe_resched()`, consulted only at the timer-IRQ tail and the syscall-exit wrapper tail, is the sole forced-context-switch decision point; wakers only raise `need_resched`.
- Task stacks are watermark-filled and verified at every switch-out; exhaustion panics with the task name.
- Every `LoadElf` request carries an argv blob whose argv0 names the task.

## Design

Interrupt discipline — kernel code runs with IF=0 except idle `hlt` and the user-return boundaries (`iretd`/`SFMASK`); the scheduler, future registry, and mailbox manager mutations are `IrqGuard` sections (`kernel/src/irq_guard.rs`) whose re-entry tripwire prints `[IRQGUARD]` and must stay silent. Event-subscribed IRQ lines are level-triggered-safe: dispatch masks and signals EOI, delivers the event frame, and **stays masked** — the driver re-arms via the `IrqClear` syscall only after clearing its device's interrupt source (unmasking while the device still asserts livelocks the CPU). Edge registrations (timer, keyboard) keep the dispatch-time auto-unmask. Forced context switches have a single site: wakers raise `need_resched` and `Kernel::maybe_resched()`, consulted only at the timer-IRQ tail and the syscall-exit wrapper tail, is the sole decision point — intent survives non-preemptible windows uncleared (`plans/stack/deferred-reschedule-impl.md`).

`elf/` — loads standalone user-space ELF binaries into tasks (`new_elf_task_with_arguments`).

`task/` + `task_manager/` — task lifecycle, context switch, preemption. Task stacks are sentinel-watermark-filled and verified at every switch-out (deep-watermark scan + saved-SP-vs-guard check sized to one swap frame): exhaustion panics with the task name — a permanent tripwire, like `[IRQGUARD]`. Every `LoadElf` request carries an argv blob whose argv0 is the program's identity, and the kernel names the task from it (`TaskName`, a 32-byte inline buffer, truncated on a char boundary) — every task carries its program name.
