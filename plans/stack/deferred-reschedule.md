# Plan 1 — Deferred Reschedule: One Forced-Switch Site

> **Status:** Implemented (`deferred-reschedule` branch — evidence log
> `deferred-reschedule-impl.md`) · **Date:** 2026-11-04 · **Code state:** `da8d54b`
> **Depends on:** nothing · **Blocks:** plan 3 (`dual-stack-entry-trampolines.md`)

## Problem being solved

Forced context switches execute *inside* the interrupt handler call chain:
`Kernel::preempt()` (`kernel/src/kernel.rs:249-256`) is called directly by the
timer handlers (`arch/x86_64/src/interrupts.rs:112`,
`arch/x86_32/src/interrupts.rs:162`). Consequences:

1. **No reschedule point at syscall exit.** When a task's syscall completes an
   IPC exchange or a future and wakes another task — the chain
   `FutureRegistry::notify → ForWakingTasks::wake_tasks`
   (`kernel/src/kernel_services.rs:26-27`) `→ Scheduler::wake_tasks`
   (`kernel/src/scheduler/scheduler.rs:141-148`) — the woken task lands on the
   ready queue while the waker keeps running until the **next timer tick**.
   IPC round-trip latency is up to 10 ms (100 Hz PIT) for no architectural
   reason.
2. **The switch site is handler-specific.** There is no single named place
   where "the kernel decides to switch." Plan 3's exit trampolines need exactly
   one such place to *move*, not to invent.
3. **Portability.** Switch-inside-handler is the least portable model. The
   mainstream shape on every roadmap platform is "entry records intent; the
   return-to-task boundary decides": x86 `ret_from_intr`-style check, ARM
   Cortex-M **PendSV** (the pattern made architectural, FreeRTOS's entire port
   model), ARM-A `eret` check, RISC-V `mret` check, m68k check before `rte`.

## Design

**The flag.** Add `need_resched: bool` to `ExecutionState`
(`kernel/src/state.rs:12-18`), under exactly the existing interrupts-off
discipline that `preemption_enabled` already lives under (`kernel.rs:182-183`,
`kernel.rs:204-207`). No atomics (UP; the plain-bool precedent is explicit in
the codebase). SMP would make it per-CPU — out of scope.

**Wakers set the flag.** `SchedulerWakerUseCase::wake_tasks`
(`kernel/src/kernel_services.rs:26`) additionally sets `need_resched`. This one
choke point covers every wake source: timer expiry, IPC completion, task
completion, keyboard futures. Extra sets are harmless (the flag is an intent,
not a count). Wakes that happen inside the scheduler task's own `step()`
(`kernel/src/scheduler/scheduler.rs:116-132`) pick the next task immediately
afterward anyway; a redundant set there is gated away by `should_preempt()`
(the quantum check never preempts a freshly-picked task).

**The exit check.** Rename/restructure `preempt()` into `maybe_resched()`:

- if `!need_resched` → return.
- clear the flag, then run today's decision body verbatim:
  `preemption_enabled && scheduler.should_preempt()` → stamp
  `YieldReason::Preempted` → `switch_to_scheduler()`.
- if `preemption_enabled == false`: **do not clear** — the intent survives the
  non-preemptible window and fires at the next check (that is what deferral
  means).

**Consult points** (the only two today):

1. Timer IRQ: tick → EOI → `mark_resched` → `maybe_resched()`. Same net
   behavior as today (EOI already precedes `preempt()`), now through the
   common decision function.
2. Syscall exit: `maybe_resched()` after `handle_syscall` returns, in both
   ports' thin Rust wrappers — `arch/x86_64/src/cpu.rs:173-176` and
   `arch/x86_32/src/cpu.rs:105-109`. **No asm/stub changes**: a switch taken at
   the wrapper tail suspends the waker inside its syscall chain, and it resumes
   through it — identical mechanics to today's mid-handler switch, just one
   frame shallower.

Keyboard IRQ is unchanged (`enqueue` only, `kernel.rs:203-208`); keyboard-driven
wakes flow through `wake_tasks` and therefore already carry the flag.

**Voluntary paths unchanged.** `task_yield`, `wait_future`,
`terminate_and_yield` (`kernel.rs:224-247`, `262-277`) are decision points in
their own right.

**Bounded-staleness invariant (documented, accepted).** A wake that lands
*after* a consult but *before* the interrupt/syscall return is missed until the
next tick. Same staleness bound as today's `preemption_enabled` windows: worst
case one 10 ms quantum blip, self-healing, never a stall.

## Non-goals

- Mechanical deferral (EOI, re-enable IF, check in an asm trampoline — the
  Linux refinement). This step makes the switch site *single*, not IF-transparent.
- IST, per-task kernel stacks (plan 3), per-CPU atomics, SMP.
- Priority inheritance.

## Hard constraints

1. Preserve the interrupts-off discipline; the flag is a plain field with the
   same protection class as `preemption_enabled`.
2. `should_preempt()` remains the sole quantum authority
   (`kernel/src/scheduler/scheduler.rs:163-165`); `maybe_resched` must not
   reimplement scheduling policy.
3. `preemption_enabled == false` windows (`schedule()`, `enqueue()`, boot,
   terminate) must never switch — the flag defers across them, never fires
   inside them.
4. `panic=abort`: no unwinding through kernel frames anywhere in this change.

## Verification gates

1. Cross-cutting gate block (`plans/stack/README.md`), including QEMU survivors.
2. New unit tests (`kernel/src/state.rs` or `kernel.rs` `#[cfg(test)]`):
   flag set/consult/clear; intent survives a disabled window; consult without
   flag is a no-op; wake_tasks sets the flag.
3. New integration behavior check: a task woken by another task's syscall is
   scheduled **before the next tick** — observable directly via `ping_client` /
   `ping_server` or `bulk_*` round-trip ordering in QEMU output (today: bounded
   by 10 ms; after: sub-tick).
4. Two busy tasks still alternate at the tick rate (timer path unchanged).

## Expected outcome

- One named decision function; all forced switches consult it; the syscall
  exit gains the reschedule point it structurally lacked.
- Measurable: IPC wake latency 10 ms-class → immediate.
- Plan 3 inherits a single, tested switch site to relocate into its exit
  trampolines instead of introducing deferral and dual stacks together.

## Open questions for the review

1. Naming: keep `preempt()` as the name of the decision function, or rename to
   `maybe_resched()` for the deferred semantics? (Recommend rename — the
   current name describes an action this function may decline.)
2. Should `Scheduler::wake_tasks` set the flag, or the `Kernel` service layer
   above it? (Recommend the service layer, `kernel_services.rs:26` — keeps
   `Scheduler` free of execution-state knowledge.)
