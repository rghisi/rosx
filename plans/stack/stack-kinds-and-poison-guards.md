# Plan 2 — Stack Kinds, Sized Allocation, Poison Guards

> **Status:** DRAFT for review · **Date:** 2026-11-04 · **Code state:** `da8d54b`
> **Depends on:** nothing (independent of plan 1; may land in parallel)
> **Blocks:** plan 3 (`dual-stack-entry-trampolines.md`)

## Problem being solved

One compile-time size serves every consumer: `STACK_WORDS = 2048`
(`kernel/src/task_stack.rs:4`) → 16 KiB resident on x86_64, 8 KiB on x86_32.
The budget assessment (stack sizing discussion, 2026-11):

- A "normal" app on this design — `core::fmt` output machinery, one IPC
  exchange, a modest stack-local buffer — has a debug-codegen worst case of
  ~8–12 KiB **before** the kernel-side syscall/IRQ frames share the buffer.
  16 KiB is at 80–100% utilization; the current apps survive because they are
  print-and-loop shapes.
- Idle and pure-kernel `fn` tasks pay the same 16 KiB they never use.
- Overflow has no detection at all: the first symptom is corrupted heap
  somewhere downstream. `plans/game-triple-fault-fix.md` is the cost record of
  debugging that failure class.

## Design

### The allocation contract

**A stack is never allocated without its guard attached.** One allocation, one
region:

```
low                                                            high
┌──────────────┬───────────────────────────────────────────────┐
│ guard words  │                   stack body                  │  ← SP at top,
│ (owned by    │   (grows down...)                             │    grows down
│  nobody)     │                                               │
└──────────────┴───────────────────────────────────────────────┘
```

Rules that become invariants of `TaskStack`:

1. Body size is a power-of-two count of `usize` words; the region's **body is
   aligned to its own size**. (This is the one genuinely forward-looking rule:
   it is free on x86, and it is exactly what ARMv8-M MPU regions and RISC-V
   PMP NAPOT entries demand later — see plan 4. Adopt it now; retrofitting it
   means reworking every stack allocation.)
2. The guard sits contiguously below the body, inside the same allocation.
   Guard size: `GUARD_WORDS = 64` (512 B on 64-bit) as a starting constant.
3. Guard filled with a magic pattern at creation; owned by nobody; a write
   there is a fault to be dispositioned, not data.

### Stack kinds (the sizing vocabulary)

```
pub enum StackKind { KernelFunction, ElfUser, Scheduler, Idle }
```

Domain knowledge stays in `task_stack.rs` (AGENTS.md: the allocating subsystem
owns identity, layout, validity). Initial defaults — named, separately
tunable numbers replacing the single anonymous one, body bytes on x86_64
(x86_32 scales by pointer width as today, or matches, once byte-budgeting —
keep `usize` words for now):

| Kind | Default body | Rationale |
|---|---|---|
| `ElfUser` | 64 KiB | The µClinux-family floor where fmt + IPC + a small local buffer stops being a coin-flip. 256 slots × 64 KiB = 16 MiB worst case, ~1.3 MiB at today's ~20-task reality, against 128 MiB QEMU. |
| `KernelFunction` | 16 KiB | Unchanged for current kernel `fn` tasks (current size suffices — verified by the survivor history). |
| `Scheduler` | 16 KiB | Runs `Scheduler::step` (`kernel/src/scheduler/scheduler.rs:116-132`) and all guard scans. |
| `Idle` | 4 KiB | Loop + `hlt` (`kernel/src/task.rs:164-170`). |

Kind selection at construction: `TaskStack::new(kind)`; `Task::new` grows a
kind parameter at its call sites (`FunctionTask::new` → `KernelFunction`,
`new_elf_task` → `ElfUser`, `idle_task_factory` → `Idle`, scheduler bootstrap
in `kernel.rs` → `Scheduler`). No KConfig wiring yet — the per-boot override is
deferred; the table lives behind one function so promoting it is mechanical.

### Representation change

`Box<[usize; STACK_WORDS]>` is compile-time sized and cannot express per-kind
sizes → **`Box<[usize]>`** (one boxed slice = guard ++ body).

- Construction stays on the zeroed path: `vec![0usize; total] →
  into_boxed_slice()` (lowers to `alloc_zeroed`; the forbidden forms from
  `plans/taskstack-extraction-plan.md` BR-2 carry over verbatim: no
  `Box::new([0; N])`, no array temporaries, no `try_into()` without the
  intermediate slice). Guard magic written afterward via slice fill (bounded
  loop, no temporaries).
- `top()` / `base()` / `size_bytes()` survive unchanged in meaning; `words()`
  test accessor survives; new: `guard_words() -> &[usize]`-equivalent and a
  pure `fn guard_intact(&self) -> bool`.

### Detection: Tier 1 poison + checkpoint

- **Check site:** `ExecutionState::switch_to_scheduler()`
  (`kernel/src/state.rs:43-60`) verifies the **outgoing** task's guard before
  `swap_context`, and `switch_to_task()` (`state.rs:22-40`) verifies the
  **incoming** task's guard (catches a blocked task whose stack was scribbled
  over while asleep). Both scans execute on the scheduler stack context — the
  checker never depends on the faulting stack being sane.
- **Cost:** 64-word scan per switch edge — noise against the switch itself.
- **Detection window:** ≤ one scheduling quantum. Attribution: which task,
  which stack; not which instruction (that is plan 4's Tier 2).
- **Guard-skip hole:** a single frame larger than the guard can jump over the
  magic. Closed today by the existing large-frame disassembly gate (frames
  bounded ≪ 512 B in reviewed code). If the gate ever relaxes, LLVM stack
  probes (`__rust_probestack` / `-Z stack-size-probes`) close it formally —
  recorded in plan 4, leaf F.

### Disposition

A violated guard is **a task fault, not a kernel panic** (`panic=abort` rules
out unwinding anyway):

1. Log: task name, stack kind, guard range, the offending values.
2. Mark `Terminated` and notify the completion future — reusing the exact
   machinery of `terminate_and_yield` (`kernel/src/kernel.rs:262-277`), but as
   a non-leaving disposition when detected at switch-out (the outgoing task is
   being suspended anyway) and via the normal termination path when detected
   at switch-in (before the task ever runs again).
3. Kernel and all other tasks continue. The victim's data lived under the
   guard; it is untouched by construction.

## Non-goals

- Unmapped guard pages, IST, MPU regions (plan 4).
- Per-task dual stacks (plan 3). The kinds above map to *today's* task classes;
  plan 3 re-reads this vocabulary as (User, Kernel) per task — keep the enum
  open to that reading.
- Stack growth, reuse pools, KConfig runtime overrides.
- Security containment (flat ring 0: guards catch SP-relative descent, not
  adversarial absolute writes — design premise, stated once, here).

## Hard constraints

1. All construction-forbidden forms of the taskstack extraction remain
   forbidden; the new `TaskStack::new` must pass the debug-codegen large-frame
   gate with a prologue at or below the current baseline (`0x178`).
2. `kernel/` stays `no_std`, dependencies unchanged (`system`, `lazy_static`).
3. Kind defaults are the single sizing truth; no call site may pass an ad-hoc
   size (a `with_body_words` escape hatch, if needed at all, is `pub(crate)`).
4. Guard magic distinct from any plausible zero/sketch content; `0xDEAD_BEEF`
   -style pattern, one constant.
5. The scheduler task's own stack keeps a guard too (it can overflow like any
   stack); its check runs before its `swap_context` too.

## Verification gates

1. Cross-cutting gate block (`plans/stack/README.md`): unit tests, debug QEMU
   survivors, disassembly gate on `TaskStack::new`, both port builds.
2. New unit tests: guard intact by default; a single write into the guard
   flips detection; POW2 + size-aligned region invariants hold for every kind;
   body stays zero-filled; guard magic written; `top()` invariant preserved
   (`base + body` after the representation change — guard excluded).
3. **Negative integration test (new failure-class gate):** an app that
   deliberately recurses/overflows its stack → kernel logs the guard-violation
   line naming the task, the task terminates, its completion future completes,
   and the remaining apps (and the shell prompt) keep running. This test is
   the plan's reason to exist: it must fail on `da8d54b` (machine dies) and
   pass after.
4. Memory arithmetic check at boot log or test: total stack bytes committed
   for the default task set; expected ≈ 20 tasks × kind default.

## Expected outcome

- The original sizing question answered *properly*: every task class has a
  named, separately tunable budget; `ElfUser` moves to 64 KiB while `Idle`
  drops to 4 KiB (net committed memory barely moves for the current task mix).
- The triple-fault failure class becomes a one-line log + dead task.
- `TaskStack` becomes the single seam where plan 3 attaches per-task dual
  stacks and plan 4 attaches sharper per-arch teeth.

## Open questions for the review

1. Kind naming: today's classes (`KernelFunction`, `ElfUser`, `Scheduler`,
   `Idle`) versus reserving those names for plan 3's (User, Kernel) reading.
   (Recommend today's names now; plan 3 renames deliberately with its own diff.)
2. x86_32 defaults: word-scaling (32 KiB for `ElfUser`) or explicit same-bytes
   policy (64 KiB)? The representation change allows either; decide at review.
3. Should the disposition mark a distinct `TaskState::Faulted` (displayable,
   distinguishable from clean exit) or log-only + `Terminated`? (Recommend
   log-only + `Terminated` now; a `Faulted` state touches the scheduler
   lifecycle plan `scheduler-lifecycle-hardening.md` and is not needed.)
