# Design Brief — Extract the task stack out of `Task` into `TaskStack`

> **Status:** PARKED. Deliberately NOT part of PR #60 (whose `MaybeUninit` in-place
> construction is approved, sound, and merged-pending). This brief records the
> structural follow-up agreed after the PR #60 review, for a future dedicated change.
> **Date:** 2026-10-14 · **Origin:** PR #60 review discussion · **Code state:** `2ad013b`

## Problem being redesigned

`Task` embeds its stack as an inline member (`stack: TaskStack` today, `[usize; 2048]`
historically — the 16 KiB buffer is the largest field by two orders of magnitude).
Rust value semantics *materialize* struct values: in debug codegen, constructing a
`Task` by literal builds ~96 KiB of temporaries on the calling task's own 16 KiB
stack before copying to the heap box. The compiler does not know those bytes are
destined to become a live stack; the OS needs a buffer that never lives anywhere but
its final address. That collision (value semantics vs. placement requirement) caused
the debug-only triple fault fixed by PR #60.

PR #60 resolves it by *reconciliation* (in-place field-wise construction — one
carefully justified `unsafe` block). This brief instead *dissolves* the conflict by
removing the giant inline member from the value type, which shrinks every `Task`
temporary below the danger threshold by construction.

## Intended design change

Move stack ownership from an inline array into a dedicated single-responsibility type
owned by the task:

```rust
pub struct TaskStack { /* owns the buffer, e.g. Box<[usize; STACK_WORDS]> */ }
impl TaskStack { fn top(&self) -> usize; /* bounds, base, size */ }

pub struct Task {
    name, state, yield_reason, stack_pointer,
    entry_point, entry_param,
    stack: TaskStack,          // owned by value inside the Task box
    completion_future,
}
```

- `Task` becomes a ~64-byte bookkeeping value: identity, scheduler state machine,
  entry point, completion future. `Task::new` returns to plain safe
  `Box::new(Task { .. })`; the `MaybeUninit` construction, the field-wise
  `ptr::write` block, and its `// Safety:` comment all disappear.
- `prepare_entry` hands `TaskStack::top()` to `Cpu::initialize_stack` exactly as
  today; the kernel/arch boundary is unchanged (both trampolines already re-align SP
  downward with `& !0xF` before writing, so ordinary heap alignment is sufficient).
- Stack size stays a word-counted truth (`STACK_WORDS`), later promotable to a
  per-arch policy (see Goals).

## Goals

1. **Remove the unsafe core**: no in-place construction, no big temporaries, plain
   safe `Task::new`; any residual unsafe isolated inside `TaskStack` (and likely
   none, via `vec![0; N]` → `alloc_zeroed`).
2. **Restore the struct-literal idiom** for the kernel's most central type, so the
   debug-codegen hazard class cannot be reintroduced by future edits to `Task`.
3. **Single responsibility**: `Task` = scheduling bookkeeping; `TaskStack` = stack
   ownership + geometry. The PR #60 SP-pins-array-top invariant test migrates onto
   `TaskStack` where it is a natural self-invariant.
4. **Open the pluggability path** required by the multi-arch goal: per-arch (and
   eventually per-task-class) stack sizes behind the HAL; future guard pages; future
   stack-allocation policy (AGENTS.md: allocator pluggability is a stated goal).
   Real-kernel precedent: Linux allocates thread stacks independently of
   `task_struct`; Zephyr separates `k_thread` from stack objects.

## Reasons (why now-worthy, why not this PR)

- The PR #60 fix is correct but permanentizes an awkward construction whose safety
  argument ("all 8 fields written before `assume_init`") must be re-verified by every
  future contributor touching `Task`. The extraction makes the hazard structurally
  impossible rather than politely documented.
- Extraction surface is unusually small (verified during the review): arch ports
  consume a plain `usize` SP and are untouched; `task_manager` uses only
  `stack_pointer()` / `stack_pointer_mut()` and is untouched; the change is confined
  to `kernel/src/task.rs` internals + unit tests.
- Not folded into PR #60: the fix there is review-approved and evidence-complete; a
  structural refactor belongs on its own branch with its own gates.

## Hard constraints (from the PR #60 investigation — do not regress)

1. **Ownership, not borrowing**: `TaskStack` must be owned by value inside the Task
   box (freeing a Task atomically frees its stack). A reference into a separately
   freed buffer would invent a dangling class the current design cannot have. The
   lifecycle guarantee transfers via the Box + `GenerationalArena` handle — exactly
   as today; nothing about handle validity changes.
2. **Do not re-import the bug inside `TaskStack::new`**: `Box::new([0usize; N])` or
   any big-array literal is the identical debug-codegen trap one level down. Safe
   forms: `vec![0usize; STACK_WORDS]` (zero-vec lowers to `GlobalAlloc::alloc_zeroed`;
   `MemoryManager` does not override it, so core's default zeroes in the allocation
   itself — no caller temporary) or a `Box::new_uninit` scoped to the array only.
3. **Regression arbiter**: re-run the debug disassembly large-frame gate
   (`sub $>=0x1000,%rsp` scan; see `plans/game-triple-fault-fix.md`) on `Task::new`
   AND `TaskStack::new`, plus the QEMU survivors (`snake_survives_idle`,
   `tetris_survives_idle`) on the debug kernel. The frame must stay small; PR #60's
   baseline is 0x178.

## Expected high-level outcome

- `Task` is a clean, small, safe-to-construct bookkeeping type; stack mechanics and
  their complexity live in `TaskStack` with one job.
- The `unsafe` inventory of the task subsystem goes to zero (or to one tiny,
  local `TaskStack` detail), and the placement hazard becomes structurally
  impossible rather than discipline-enforced.
- No behavior change: identical SP values, zero-fill extent, and arch trampolines;
  one extra heap allocation per task (negligible for ≤256 tasks).
- Stack sizing becomes a natural place to attach arch policy and, later, guard
  pages / a stack allocator — without touching `Task` again.

## Out of scope (future considerations)

Guard pages, stack growth, and a dedicated stack allocator; per-arch size selection
mechanism (trait method vs. `KConfig` field) — decide during the dedicated design.
