# Scheduler & Task-Lifecycle Hardening (deferred from the game triple-fault fix)

> **Status:** future work — deliberately NOT part of the memory-management fix.
> **Origin:** run `fix/game-triple-fault` (2026-10-11/12). These items were implemented,
> unit-tested green (182 → 191), and warning-neutral (Δ-new = 0) on the branch, but
> forensics proved none of them caused the snake/tetris triple fault (the convicted root
> cause is the free-list allocator; see `plans/game-triple-fault-fix.md` Rev-4..R7 + erratum).
> The user directed a memory-only ship, so the whole bundle was reverted from the tree and
> parked here.
> **Code snapshot:** `plans/scheduler-lifecycle-hardening.patch` — verified to apply cleanly
> to `97be6b7` (branch `fix/game-triple-fault`). Rebase like any normal change.
> **Full specs** of each item live in `plans/game-triple-fault-fix.md` (Rev-1 Steps 1–2,
> Rev-3 §FIX-E) and the per-item notes below.

## Bundle inventory (what the patch contains)

### 1. MLFQ stale-handle hygiene — P1
- `push_ready` dedups via `contains_handle` before inserting; `on_task_terminate`
  removes the handle from **all** queues and returns yield_info consistently.
- Why it matters: duplicate/stale ready-queue entries were **observed live during boot**
  (KernelCell re-entrancy depth 2 at boot, ≥4 during gameplay) even though they were
  exonerated as the fault cause. A terminated task surviving in a ready queue is a real
  correctness hazard (use-after-recycle of arena slots).
- Tests: 5 new unit tests (dedup, terminate-removes-all, mixed-queue states).

### 2. Loud switch path (D2) — P1
- `get_task_stack_pointer() -> Option<usize>` (no silent `0` for missing/dead tasks).
- `switch_to_task` defensively skips dead/missing handles **and** carries a
  `debug_assert!` so a stale handle is loud in debug builds instead of restoring SP=0.
- Why it matters: the old SP-0 fallback path was a decoy that cost a full forensics round;
  making the failure mode loud short-circuits the entire bug class in debug runs.
- Tests: 2 new unit tests.

### 3. `swap_save_slot` staging on `ExecutionState` (FIX-E) — P3
- Replaces the raw `execution_state.swap_context` aliasing with a dedicated
  `swap_save_slot` (set at `switch_to_task` time, consumed by the asm save path).
- Honest status: the save-side arena-aliasing theory (M1) was **refuted** — with FIX-E the
  corpse byte-identical. Keep-or-drop is a design call: it removes one raw-pointer
  double-buffer hazard and makes the swap contract explicit; it is not required for
  correctness today. If applied later, re-verify the asm handoff in
  `arch/x86_64/src/context_switching.S` is untouched (USER-GATE: arch changes need explicit
  user approval).

## Related follow-ups from the same investigation (not in the patch)

### 4. FIFO strategy dedup gap — P2
- `fifo_strategy` has the same push/terminate shape as MLFQ without the dedup added in
  item 1. Apply the same hygiene once MLFQ's version is merged, for strategy parity.

### 5. `wake_tasks` guard against waking Running tasks — P2
- The wake path does not check the task's current state; a wake racing with a
  still-Running instance can double-enqueue (the exact shape item 1 defends against,
  upstream of the queue).

### 6. KernelCell re-entrancy — P2 (architecture item, needs design pass first)
- Measured facts from the scratch instrumentation: re-entrancy depth 2 at boot, high-water
  ≥4 during gameplay (futures/IPC paths re-enter the kernel cell from within kernel-cell
  work). Currently "works" but is latent-UB territory (nested `&mut` aliasing). Needs a
  design proposal (queue-based re-entry or explicit re-entrant-safe state) before any code.

### 7. User `Dealloc` syscall pointer validation (F3 hardening) — P1
- `kernel/src/syscall.rs` `Dealloc` passes an unvalidated user pointer into the kernel
  allocator. R7's allocator fix rejects bad frees **inside** the allocator, but the proper
  boundary check (pointer belongs to the caller's owned user-heap range / owner-tagged
  `deallocate_by_owner` path) belongs at the syscall boundary. R7 explicitly deferred this
  ("do not widen this fix"). The allocator `BAD-FREE` log will name the first offender —
  file the ticket against that evidence.

### 8. Remove dead `Task::stack_pointer_mut` — P3
- `kernel/src/task.rs` `stack_pointer_mut` has zero callers; delete or document.

### 9. rflags/`sti` handoff hygiene in context switch (ex FIX-C) — P3, USER-GATE
- The `sti`-ownership asymmetry between kernel and task-context switches is latent but was
  formally **demoted to hygiene** (R5.3/R7): it is not a fault cause. Any change here
  touches `arch/` and requires explicit user approval per the standing repo gate.

## Verification posture to restore when picking any item up

- Baseline before patch at `97be6b7`: `cargo test -p kernel -- --test-threads=1` → **182 passed**;
  `cargo test -p collections` → 21. Patch on top → **191 passed** (verified 2026-10-11).
- Warning bar: Δ-new = 0 vs fingerprint `{kernel lib 65, rosx bin 26, shell lib 5, +1 manifest}`,
  counting both `warning:` and `warning[EXXXX]:` forms.
- Parity: `cargo build -p rosx-x86` from `arch/x86_32/`; `cargo xtask build --debug`.
- Kernel `no_std`, no comments, no new dependencies; `arch/` untouched unless user-gated.
- If applied while `fix/game-triple-fault` work is still live, re-run the §R2.2 game repro
  (snake + tetris) to prove the bundle remains fault-neutral.
