# Plan 3 — Per-Task Dual Stacks with Software-Switched Entry Trampolines

> **Status:** DRAFT for review · **Date:** 2026-11-04 · **Code state:** `da8d54b`
> **Depends on:** plan 1 (single forced-switch site) and plan 2
> (kinds + guards; the frame-stays design below relies on guards bounding
> overflow). **Blocks:** most plan-4 leaves.

## Problem being solved

There is no user stack: `load_elf` (`kernel/src/elf/mod.rs`) allocates only
image memory, and `elf_task_wrapper` (`kernel/src/task.rs:319-333`) invokes
the user entry **as a function on the kernel-allocated `TaskStack`**. The same
buffer serializes all four cost centers — app frames, the syscall trampoline
(`arch/x86_64/src/syscall.S` pushes 15 registers on the user's own stack; the
x86_32 `int80_handler`, `arch/x86_32/src/cpu.rs:163-185`, likewise), IRQ
frames (no IST on either port; the frame lands on whatever the 100 Hz tick
hits), and context-switch pushes. Consequences: sums instead of separate
budgets, kernel frames destroyable by app overflow, `argc/argv` passed as a
kernel-heap pointer through a register (`ElfTaskContext`, `task.rs:176-224`)
instead of the ELF-ABI initial stack, and kernel wrapper code with its own
failure modes living inside the app's budget.

Design premise (user decision, 2026-11): **everything stays ring 0**; no MMU.
The mechanism is therefore software-switched entry stubs — not a placeholder:
this is the steady-state form on x86 at CPL0, and on m68k the same model is
native (USP/SSP hardware mode-switch), so designing to it pays across the
roadmap.

## Design

### Ownership and slots

- `Task` owns `kernel_stack: TaskStack` (kind `Kernel`) and
  `user_stack: Option<TaskStack>` (kind `User`). ELF tasks get both;
  `fn`-tasks, `Idle`, and the scheduler task get kernel-stack-only (their
  "user" execution is kernel code). Ownership stays by-value in the Task box
  (hard constraint inherited from the taskstack extraction — no reference into
  a separately freed buffer).
- **No new Task SP field.** The saved user SP lives where it belongs: in a
  per-entry record on the task's own kernel stack (see protocol). The one SP
  the scheduler migrates remains `Task::stack_pointer`, pointing into the
  kernel stack at suspension. `swap_context` keeps its exact signature on
  both ports.

### Entry/exit protocol (both ports, all four current entries)

Arrival model, flat ring 0: the CPU pushes the same-privilege `iret` frame
(`[RIP, CS, RFLAGS]`; `[EIP, CS, EFLAGS]` on 32-bit) onto the **user stack**
before any stub code can run — software switching cannot prevent this, and we
do not fight it. **The frame stays there.** The kernel keeps only the address
of it.

```
stub entry (still on user stack, RSP = frame):
    save current RSP (= user frame pointer)        ; into record
    load current task's kernel-stack top           ; via KERNEL_CURRENT, below
    push [exit_user_trampoline_addr][saved_user_RSP]  ; the record, on kernel stack
    set RSP into kernel stack
    push callee-saved; jump into the common Rust handler chain
    ... handler runs entirely on the kernel stack;
    ... timer marks + consults via plan 1's decision fn (on the kernel stack)

exit_user_trampoline (reached by the unwinding kernel return chain):
    pop saved_user_RSP -> RSP
    iret                                            ; frame never moved
```

Works because nothing else ever touches that user stack while the task is
elsewhere or suspended — which is only true *because* plan 2's guards bound
every other stack's overflow. Resume after a taken switch is transparent: the
suspend chain is `[user stack: hw frame][kernel stack: record + handler chain
+ swap_context pushes]`, migrates with the task, unwinds normally, and lands
at `exit_user_trampoline`. Same for the timer IRQ, the keyboard IRQ, and the
x86_32 `int80` (hardware already pushed its frame — nothing to synthesize).

**SYSCALL (x86_64, no hardware frame):** the LSTAR trampoline
(`arch/x86_64/src/syscall.S`) synthesizes the same shape while still on the
user stack — push `R11` (RFLAGS), kernel CS, `RCX` (RIP) — then proceeds with
the identical record protocol. Exit is uniformly `iretq`; `sysretq` stays a
later optimization. This also removes the odd dual-path in the current IDT
(vector 0x80 retargeted by transmute, `arch/x86_64/src/interrupts.rs:47-50`).

**Kernel-current lookup for stubs:** the kernel exposes a fixed,
arch-readable symbol (e.g. `CURRENT_KERNEL_SP_TOP`) updated by
`ExecutionState::switch_to_task` / `switch_to_scheduler`
(`kernel/src/state.rs:22-60`) — both already know the incoming/outgoing task.
Data ownership stays kernel-side (AGENTS.md); archs only read one address,
the same linking idiom already used for `swap_context` /
`syscall_handler_entry` (`arch/x86_64/src/cpu.rs:168-171`).

### Entry composition: wrappers on the kernel stack; a real ELF initial stack

- `task_wrapper` / `elf_task_wrapper` run on the kernel stack. For ELF tasks
  the wrapper: loads the image, builds the System-V initial stack at the top
  of the user stack — `argc`, `argv[]`, `NULL`, `envp[]`, `NULL`, 16-byte
  aligned — then enters user via a one-time jump: push sentinel return
  `task_exit_trap`, set RSP below the args, `jmp entry`.
- `task_exit_trap` (arch asm): the landing site when user `main` returns; it
  re-enters kernel-side and calls the terminate path (today's
  `terminate_and_yield`, `kernel.rs:262-277`).
- **Image lifetime fix:** today `elf_task_wrapper` drops the image `Vec`
  (`task.rs:329-330`) *after* main returns — impossible in the jump model.
  The image must be retained until task termination (store it with the task's
  termination bookkeeping and free it in the terminate path). No leak
  regression: termination of an ELF task frees image + argument strings.
- `ElfTaskContext` (`task.rs:176-224`) and its heap-pointer argv hack are
  **deleted**. The initial-stack convention is part of the boundary treaty:
  record it in `system` alongside the syscall numbers/ROM layout.
- **usrlib ABI flip, atomic with this plan:** `usrlib/src/runtime.rs` entry
  takes `argc/argv` from the stack instead of registers. Old kernel + new
  apps is a broken combination by construction — gate with `cargo xtask build`
  + ROM + the integration suite in one landing.

### Guard checks follow the stacks

Plan 2's switch-time scan covers every stack the task owns (`user_stack` when
present, `kernel_stack`, scheduler task's own). Dispositions already built in
plan 2 now attribute correctly: user overflow kills the app; kernel-path
overflow names the kernel stack.

### Kernel-stack sizing becomes a measured question

New sub-task of this plan: **paint the kernel stack and read the high-water
mark** across the survivors + IPC apps in QEMU, then set the `Kernel` kind
default from data instead of guess (expected 8–16 KiB). The user stack keeps
plan 2's `ElfUser` default (64 KiB). Total worst case at 256 slots:
64 + 16 KiB-class per task ≈ ~20 MiB — unchanged order against 128 MiB QEMU,
and right-sized per class.

## Non-goals

- Ring 3, TSS, per-task page tables, guard pages (plan 4), MPU (plan 4).
- IST for timer/keyboard. Running IRQs on shared IST stacks requires full
  deferred switch-from-trampoline with frame *migration* — the frame-stays
  model above is incompatible with that by design; if IST-for-IRQ is ever
  wanted, its frame-migration variant is a successor plan (option B below is
  its seed).
- `sysretq` fast exit; signals-style frame injection; user-stack growth; SMP.

## Hard constraints

1. Plans 1 and 2 landed and green (single switch site; guards bounding all
   stacks — the frame-stays premise).
2. `swap_context` signatures unchanged; one migrated SP per task.
3. Masking semantics unchanged: IF stays masked from entry through decision to
   `iret` (as today; `SFMask` at `arch/x86_64/src/cpu.rs:52` preserved); PIC
   EOI remains before the reschedule decision. Kernel entries do not nest —
   the record-per-kernel-stack design depends on it; do not add
   `sti`-mid-handler anywhere in this change.
4. 16-byte stack alignment restored at each stub boundary (precedent:
   `initialize_stack` masking at `arch/x86_64/src/cpu.rs:80`); `disable-redzone`
   on the kernel target stays as-is (`arch/x86_64/rosx.json:13`).
5. Nothing arch-specific in `kernel/` — the arch contract is: read
   `CURRENT_KERNEL_SP_TOP`, provide `exit_user_trampoline` /
   `task_exit_trap` / user-entry jump, update the current-SP symbol from
   `ExecutionState`.
6. Taskstack-extraction construction-forbidden forms carry over (two stack
   allocations per task now — both on the `alloc_zeroed` path).
7. Edge case, accepted semantics: if an IRQ arrives while the user stack is
   within 3 words of its bottom, the hardware frame itself lands in the guard.
   The switch-time scan catches the violation; the task is dispositioned as an
   overflow. (An app one word from full is an app that dies this quantum
   anyway; document, do not special-case.)
8. `panic=abort` everywhere; no unwinding through stubs.

## Verification gates

1. Cross-cutting gate block (`plans/stack/README.md`) incl. debug QEMU
   survivors and the disassembly frame gate on every touched constructor.
2. Existing app suite as regression for all four rewritten entries:
   `snake`/`tetris` (IRQ entries via keyboard + timer), `ping_*`/`bulk_*`
   (syscall entries both directions), `vfs_*` (IPC + ROM path), `test_suite`.
3. New negative tests: user-stack overflow mid-IPC → app dispositioned,
   **counterpart task and kernel-stack guards intact**, machine survives;
   deliberately deep kernel syscall path → names `kernel` stack as culprit.
4. New positive test: `argc/argv` observed from the stack by an app
   (`hello_elf` extended with argv echo).
5. Kernel-stack high-water report produced once and recorded in this file at
   implementation time, justifying the `Kernel` kind default.

## Open questions for the review

1. **Frame-stays (recommended) vs frame-copies.** Copying the 3-word frame
   into the kernel record keeps the kernel self-contained (needed for ring-3
   `sp0`-style entry and signal delivery someday) at the cost of writes on
   every IRQ. Recommend frame-stays now; record-copy explicitly reserved as
   the successor seed if IST-for-IRQ or signals ever arrive.
2. **Uniform `iretq` exit vs `sysretq` for SYSCALL** (recommended: uniform
   `iretq`; simpler, identical to IRQ path; `sysretq` is a measurable
   optimization for later).
3. **usrlib ABI flip scope**: full `argc/argv/envp` vs `argc/argv` only for
   now (recommend `argc/argv` + empty `envp`, treaty notes updated in `system`).
4. Does `test_suite` grow the overflow injection cases, or do they live as
   QEMU integration tests only?
