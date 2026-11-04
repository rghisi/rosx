# Plan 2 Implementation — Stack Kinds and the Poison Guard Word

> **Status:** IMPLEMENTED · **Date:** 2026-11-04 · **Code state:** built on `ab18a1a`
> **Plan:** `stack-kinds-and-poison-guards.md` (drafted at `da8d54b`)
> **Note:** the review deliberately executed a lighter contract than the draft
> in two places (guard size, disposition); both recorded below.

## Stale premises in the draft (corrected before execution)

- `STACK_WORDS = 2048` and "overflow has no detection at all" were overtaken:
  the watermark-sizing work had already raised the single size to 8192 words
  (x86_64) / 12288 (x86_32) and installed a switch-out tripwire (deep-watermark
  scan + saved-SP-vs-zone) that **panics**. The plan's negative-test premise
  (machine dies on overflow) still held — detection existed, containment did not.
- Plan 1 (deferred reschedule) landed while the plan waited: the single switch
  site is now `Scheduler::step` → `KernelContextSwitcher::switch_to_task`,
  which is exactly where the guard checks attach.

## What was built

### Representation

`TaskStack` is one allocation, sized by `StackKind` in **bytes** (same budget
on both ports):

| Kind | Body | Rationale |
|---|---|---|
| `ElfUser` | 64 KiB | covers the `vfsd`→`atad` format chain (~35 KiB measured) |
| `KernelFunction` | 16 KiB | kstatsd (`Kernel::wait` + IPC frames) |
| `Scheduler` | 16 KiB | step + reconcile + drop glue |
| `Idle` | 8 KiB | the budget is the timer IRQ tail + `maybe_resched()` on the idle stack, not the `hlt` loop (draft said 4 KiB — it undercounted plan 1) |

Region layout (one `alloc_zeroed`, `region_bytes = 2 * body + word`):

```
region                                                    region end
  ┌─ alignment slack ─┬─ guard word ─┬──── aligned body ─────┐
  │   (zeroed slack)  │ 0x5EED_1234  │  STACK_WATERMARK fill │  ← SP at top
  └───────────────────┴──────────────┴───────────────────────┘
                        body-8         body (aligned to its own size)
```

- **Invariant 1 kept:** the body is a power-of-two byte size and aligned to its
  own size (`base % body_bytes == 0`), unit-tested for every kind — the
  ARMv8-M / RISC-V PMP hook from plan 4.
- The draft's `vec![…] → into_boxed_slice()` cannot deliver size-alignment;
  construction uses `core::alloc::alloc_zeroed` + `Drop` (documented-unsafe,
  small). BR-2 survives: no array temporaries, bounded fills only; the debug
  prologue of `TaskStack::new` is `sub $0xe8,%rsp` (≤ the `0x178` baseline).
- Kind selection: `FunctionTask::new` → `KernelFunction`, `new_elf_task*` and
  `new_entrypoint_task` (Spawn siblings run user entry points — the draft's
  table omitted it) → `ElfUser`, `idle_task_factory` → `Idle`, scheduler boot
  task → `Scheduler`. No KConfig wiring, one `StackKind::body_bytes()` truth.

### Guard and detection (lighter than the draft, by review)

- The draft's 64-word guard region is reduced to **one magic word** at
  `body - 8`, `0x5EED_1234`, shared via `system::stack::STACK_GUARD_MAGIC`.
- **Check site:** only when switching **to** a task —
  `KernelContextSwitcher::switch_to_task` verifies the scheduler task's guard
  and the incoming task's guard before `swap_context`; `Kernel::start` checks
  the scheduler task before the boot swap. Detection window: the violator
  cannot run again undetected (caught before its next dispatch).
- **Disposition:** panic (system halt), not the draft's log-and-kill; the
  existing switch-out watermark/floor observation keeps its identical panic
  role (zone renamed `SpillIntoGuard` → `SpillIntoFloor`, since the guard is
  now the word below the body). Message: `stack overflow: task '<name>'
  (<Kind>) guard word at 0x… overwritten (found 0x…, magic 0x…)`.
- The body keeps the watermark fill (the draft's "zero-filled body" gate was
  overtaken by the watermark instrument; `is_exhausted` needs it).

### Fault-injection gate

`apps/boom` (new ROM app): scans down from its stack pointer for
`STACK_GUARD_MAGIC` (the body it descends through is watermark-filled, so the
first hit is its own guard), overwrites exactly that one word, and sleeps —
no heap damage below the guard. `tests-integration/tests/stack_guard.rs`
asserts the corruption line, then `!!! PANIC !!!` naming the task and the
guard address. It fails on pre-change kernels (no guard word exists; the
machine keeps running and the line never appears).

## Verification (2026-11-04)

- `cargo test -p kernel -- --test-threads=1`: 391 passed (new: kind table,
  POW2 + size-alignment per kind, guard survives/corrupts, floor observations).
- `cargo xtask build` (x86_64) + `cargo xtask build x86_32`: both green;
  `cargo xtask check-isa x86_32`: kernel + 28 user ELFs 486-clean.
- Large-frame gate: no `sub $≥0x1000,%rsp` in the debug kernel; `TaskStack::new`
  prologue 0xe8.
- Negative integration test on x86_64: **passes**, console evidence:
  `stack overflow: task 'ELF' (ElfUser) guard word at 0x20000c2fff8
  overwritten (found 0xa112edcb, magic 0x5eed1234)` — body base `0x20000c30000`
  (64 KiB aligned ✓).
- Full x86_64 + x86_32 integration suites: recorded in the commit.

## Follow-ups (not this plan's scope)

- The word guard inherits the draft's guard-skip caveat: an untouched large
  frame could jump over it; the floor/watermark observation on switch-out
  covers the same condition from the other side. Formal close: plan 4 leaf F
  (stack probes).
