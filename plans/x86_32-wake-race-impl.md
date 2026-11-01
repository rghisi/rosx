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

### M0 — Reproducible baseline (no kernel change) — DONE 2026-11-01

Harness ran with the guard in `kstats_mem.rs` temporarily removed; the tree
was restored afterwards (the commit history of the test file is untouched).

Environment: host x86_64, QEMU 10.2.1 (`qemu-system-i386`), tree
`x86_32-wake-race` @ `f43c277`, user ELFs + ROM from `cargo xtask apps
x86_32`, kernel built by the harness (`ROSX_QEMU_PROFILE` selects profile).

| profile | kernel sha256 (prefix) | rounds | pass | fail |
|---|---|---|---|---|
| debug (`target/rosx-i686/debug/rosx-x86_32`)   | `05fde23e` | 6 | 1 | 5 |
| release (`target/rosx-i686/release/rosx-x86_32`) | `9408f528` | 6 | 6 | 0 |

Failure signature (all 5 failing debug rounds, rounds 1/2/3/5/6): the shell
echoes `mem`, then `timed out after 30s waiting for output containing
"KSTATSD memory snapshot"` — the parked receiver never resumes, exactly the
parent document's symptom. Release round logs show the full snapshot report
and the return to `rose>`.

Note against the parent document: the failure rate measured here is 5/6, not
~100%; the parent doc's harness was the pre-guard merged tree run directly.
Repro is still overwhelming and profile-dependent, which is what M2
instrumentation needs.

Gate: PASSED — the race reproduces reliably in debug and is masked in release.

### M0.5 — Stack-overflow hypothesis (user-raised) — REFUTED as the mechanism, 2026-11-01

Hypothesis: the failures are task-stack overflow silently corrupting adjacent
kernel-heap blocks (no guard page, `KernelCell` has no borrow tracking), which
would mimic a lost wake. Legitimate precedent: the 8 KiB → 16 KiB bump fixed a
real overflow-of-this-kind before.

Step 1 (falsifier, no instrumentation): `STACK_WORDS` 4096 → 16384 (64 KiB)
for 32-bit, debug profile, same harness/machine: **10/10 PASS** vs the 16 KiB
1/6. Suggestive.

Step 2 (mechanism probe — temporary, fully reverted): task stacks filled with
a poison word; low-water depth scanned before every swap-out (so every stack
write is witnessed at the writer's next switch — an overflow at `base()` is
structurally impossible to miss); crossings at 25/50/75% and `OVERFLOW`
printed with the task name.

| build | rounds | result | max depth observed | OVERFLOW lines |
|---|---|---|---|---|
| 16 KiB + probe | 6 | 0 pass / 6 fail | 9164/16384 B (56%, an `ELF` task at park) | **0** |
| 64 KiB + probe | 1 | 1 pass | none crossed 25% of 64 KiB (= 16 KiB) | 0 |

`[K] Kstatsd` (the reply sender) never crossed even 25% of 16 KiB (4 KiB);
`[K] Main Thread` peaked at 4356 B. Failing rounds top out at ~9.2 KiB of the
16 KiB — no task ever wrote below its base.

Verdict: overflow is not what corrupts the wake in this failure. The 10/10 at
64 KiB is luck of heap layout / timing, which is exactly the layout
sensitivity the parent document describes for the true lost-wake race. The
stack size stays at 16 KiB; the investigation proceeds with M1 (IF/contract
diff) and M2 (wake-path counters + re-entrancy detector).

Artifacts: `/tmp/opencode/m05-*.log` (round logs incl. console dumps),
`/tmp/opencode/m0-baseline.sh`. All probe code removed from the tree.

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
