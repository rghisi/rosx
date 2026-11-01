# x86_32 wake-path race — implementation plan

Branch: `x86_32-wake-race`, forked from `origin/main` @ `cb00d39`.
Parent document: `plans/x86_32-wake-race.md`. Goal: prove the lost wake, fix the
window, unguard `kstats_mem` on x86_32.

## Corrections to the parent document (from code review at `cb00d39`)

1. The `sti; ret` resume window is **not** x86_32-only. Both ports end
   `swap_context` with `sti; ret` (`arch/x86_32/src/cpu.rs:145`,
   `arch/x86_64/src/context_switching.S:40`). SFMASK masks only the x86_64
   *entry*; a task resumed on x86_64 also runs post-swap bookkeeping with IF=1.
   What differs between the arches is the window's size, the tick source, and
   what an IRQ can find inside it — not its existence. Any fix therefore has to
   be correct for both arches.
2. `KernelCell` (`kernel/src/kernel_cell.rs`) is an `UnsafeCell` with no borrow
   tracking: `borrow_mut()` hands out `&mut` freely, so an IRQ mutating a
   structure a preempted task was mutating mid-syscall corrupts it silently.
   The observed symptom (hang, no panic, debug-only) fits this exactly.
3. The IRQ mutates scheduler state on every tick:
   `timer_interrupt_handler` → `Kernel::preempt` →
   `services().scheduler.borrow_mut().should_preempt()` mutates
   `MlfqStrategy.remaining_quantum`, and `apply_pending_preemption` mutates
   `ExecutionState`, task states, and the run queue
   (`kernel/src/kernel.rs:246-259`, `kernel/src/scheduler/scheduler.rs:99-179`).
   Because `swap_context` resumes with IF=1, that re-entrant mutation can land
   on top of:
   - the scheduler thread's own handoff bookkeeping
     (`reconcile_returned_task` → `start_next_task` → `pick_next` →
     `set_state(Running)`, all running with IF=1 between swap_context's `sti`
     and the next swap's `cli`), and
   - a kernel-task sender's `wake_tasks` → `set_state(Ready)` +
     `push_ready` (`kernel/src/scheduler/scheduler.rs:117-124`), which execute
     with IF=1 in kernel-task context.
   A corrupted MLFQ queue explains the measured trace: `mem` is woken into
   Ready yet never picked again.
4. The notify chain also splits across separate KernelCell borrows:
   `MailboxManager::notify_waiters` runs `complete_ipc_message(fh, msg)` and
   `notify(fh)` as two operations (`kernel/src/ipc/mailbox_manager.rs:93-109`),
   interruptible between them in kernel-task context.

## Milestones

### M0 — Reproducible baseline (no kernel change)

1. `cargo xtask build x86_32 --debug` and release build.
2. `ROSX_ARCH=x86_32 cargo test -p tests-integration --test kstats_mem -- --test-threads=1`
   — temporarily drop the guard skip so the test actually runs:
   - debug profile, 6 rounds → expected 0/6.
   - release profile, 6 rounds → expected 6/6.
3. Record the table (QEMU version, kernel hash, round results) in this file.

Gate: failure reproduces deterministically in debug.

### M1 — Contract diff of the wake paths (doc only)

Table, one row per step, columns: step | IF state x86_32 | IF state x86_64 |
KernelCell structures mutated | IRQ-reentrant? Covering:

- park: `receive_message_async` syscall → `pop_front_async` → `wait_future`
  (`is_future_completed` → `block_current_task` → `register_waiter` →
  `switch_to_scheduler` → `swap_context`).
- notify (user sender and kernel-task sender): `send_message`/`reply` →
  `push_back` → `notify_waiters` → `complete_ipc_message` → `notify` →
  `wake_tasks` → `set_state(Ready)` + `push_ready`.
- kernel-originated: `deliver_to_service` from the keyboard IRQ.
- scheduler handoff: `step` → `pick_next` → `set_state(Running)` →
  `switch_to_task` → `swap_context` → `sti; ret` → post-swap bookkeeping →
  syscall/IRQ epilogue.
- timer tick: entry gate IF behavior, `preempt`, `apply_pending_preemption`.

Append the per-arch IF-state conclusion table to this file. Output must state
where the arches actually diverge (refutation or confirmation of correction 1).

### M2 — Instrumentation; prove the lost wake (temporary, reverted in M4)

Counters printed to the debug console, all tagged with the handle they act on
and `are_interrupts_enabled()` at that point:

- `park(future)` in `wait_future` / `wait_waitset`,
- `notify(future)` in `FutureRegistry::notify`,
- `wake(handle)` in `Scheduler::wake_tasks`,
- `pushed(handle)` in `push_ready`,
- `picked(handle)` in `start_next_task`,
- `swapped-in(handle)` after `swap_context` returns.

Re-entrancy detector: a bit set while `Scheduler` (algorithm + ExecutionState)
or `FutureRegistry`/`MailboxManager` is being mutated, checked at the entry of
every IRQ-driven mutation (`preempt`, `apply_pending_preemption`,
`deliver_to_service`); print `REENTER` with the structure name when it trips.

Reproduce once. Decision matrix:

| observation | verdict |
|---|---|
| `pushed(mem)` yes, `picked(mem)` never | run-queue corruption through the IF window (suspect 1 family) |
| `wake(mem)` never, `notify(fh)` yes | lost wake inside notify/waiter registration (suspect 2) |
| `REENTER` fires before the hang | re-entrant `&mut` corruption; the structure name localizes the fix |

Gate: one instrumented run explains mechanically why `mem` never resumes.

### M3 — Fix, one change at a time

Ordered by likely evidence; stop when M2's verdict is addressed:

1. **IF discipline:** remove `sti` from `swap_context` on x86_32 (and x86_64 if
   M1 shows it is needed for correctness there too); re-enable interrupts only
   at the true boundaries — int80 `iretd` (already restores user IF via the
   saved eflags), the syscall-return path, the idle loop before `hlt`, and the
   scheduler-thread top loop. Kernel bookkeeping after a resume stays IF=0.
   Verify: debug `kstats_mem` 6/6.
2. **Notify atomicity:** make `push_back` → `notify_waiters` →
   `complete_ipc_message` → `notify` → `wake_tasks` one IRQ-atomic region
   (interrupts disabled around the sequence in kernel-task context), if M2
   shows a notify-side loss.
3. Any residual corruption the re-entrancy detector still shows gets its own
   smallest fix, discussed and approved before coding.

Each numbered fix is its own commit and must keep the x86_64 suite green.

### M4 — Verification and unguard

1. Remove all M2 instrumentation and the re-entrancy detector (or keep the
   detector only if a production caller exists per the no-test-only-surface
   rule — default is remove).
2. Debug x86_32: `kstats_mem` 6/6; then the full suite
   `ROSX_ARCH=x86_32 cargo test -p tests-integration -- --test-threads=1`.
3. x86_64 full suite (must not regress).
4. Keystroke variant: `python3 arch/x86_32/echo-stress.py 30 10` clean.
5. Remove the guard in `tests-integration/tests/kstats_mem.rs`; update
   `plans/x86_32-wake-race.md` (fixed) and the interim-state bullet in
   `AGENTS.md`.

## Definition of done

`kstats_mem` runs unguarded on debug x86_32 at 6/6, both arches' full
integration suites pass, `echo-stress` 30 rounds clean, no instrumentation
left, parent document and AGENTS.md updated.
