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

## Architecture decision (2026-11-01): formal locks over raw IF manipulation

Question raised: replace the IF-discipline fix with proper concurrency
handling (mutexes, semaphores)? Analysis:

- RosX is uniprocessor; the only concurrent mutator is the interrupt handler
  re-entering task-context structures. Sleeping locks cannot serve IRQ
  context, and a task-held spinlock without preemption control deadlocks when
  its holder is preempted. The correct classical primitive for this model is
  the irqsave spinlock: save IF, `cli`, restore on release — interrupt masking
  is not replaced, it is *encapsulated*.
- The codebase already converged on this pattern implicitly:
  `memory_manager.rs` hand-rolls `are_interrupts_enabled → disable → op →
  enable` around every allocator entry via the `Cpu` HAL trait.
- A general mutex/semaphore framework is rejected: most of it is ceremony
  around IF masking on UP, the sleeping-synchronization layer already exists
  as futures/waitsets (this kernel's semaphore analogue), and it would add
  deadlock classes the current model structurally cannot have
  (termination-while-held, priority inversion).

Decision — Option C, adopted incrementally and evidence-driven:

1. Keep the arch-level IF-boundary fix (interrupts enabled only at true
   boundaries). This shrinks the set of critical sections that need locks to
   "sequences an IRQ can interrupt in kernel-task context". No lock layer can
   cover context-switch assembly; IF=0 across kernel bookkeeping remains the
   invariant that makes lock scope finite.
2. Introduce one tiny primitive — `IrqGuard`: save/disable via the `Cpu`
   trait, restore on `Drop`, plus a per-structure held-flag whose re-entry
   triggers a loud failure. This is the M2 re-entrancy detector promoted to
   production; nothing built for the investigation is wasted.
3. Wrap only the structures M2 proves re-entrant. If SMP ever arrives, the
   migration surface is the guard's internals, not its call sites.

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

### M1 — Contract diff of the wake paths (doc only) — DONE 2026-11-01

Scope covered: park, notify (user sender, kernel-task sender, keyboard IRQ),
scheduler handoff, timer tick, per-arch IF state at each step.

IF-state contract per step (both arches):

| step | IF x86_32 | IF x86_64 | KernelCells mutated | IRQ-reentrant |
|---|---|---|---|---|
| fresh syscall entry | trap gate + `cli` → 0 | `syscall` + SFMASK → 0 | — | no |
| park: `receive_message_async` (pop_front / waiter register) | 0 | 0 | `mailbox_manager`, `future_registry` | no |
| park: wait (`is_future_completed` → `block_current_task` → `register_waiter` → `switch_to_scheduler` → swap) | 0 throughout | 0 throughout | `future_registry`, `task_manager`, `execution_state` | no |
| kernel-task context (kstatsd loop, any reply send) | **1** | **1** | all it touches | **yes** |
| notify: `push_back` → `notify_waiters` → `complete_ipc_message` → `notify` → `wake_tasks` → `push_ready` | **1** | **1** | `mailbox_manager`, `future_registry`, `task_manager`, `scheduler` | **yes** |
| keyboard IRQ entry | interrupt gate → 0 | interrupt gate → 0 | — | (is the IRQ) |
| timer IRQ entry (PIT / PIC or APIC; x86_64 EOI before handler, mask-unmask around) | 0 | 0 | — | (is the IRQ) |
| post-swap bookkeeping on a resumed task (state.rs epilogue, `wait_future` consume, syscall epilogue up to `iretd`/`popfq`) | **1** (`sti; ret`) | **1** (`sti; ret` — SFMASK masked only entry) | whatever the resume path runs | **yes** |
| scheduler thread post-swap bookkeeping (`reconcile_returned_task` → `start_next_task` → loop) | **1** | **1** | `task_manager`, `scheduler` | **yes** |

Conclusion on correction 1: confirmed — the two arches' IF contracts are
structurally **identical**; divergence is quantitative only (frame sizes,
window widths, tick source jitter). x86_64 shares every window below and has
merely never lost the timing lottery. The fix must be arch-neutral.

Confirmed re-entrancy / aliasing windows (targets for M2 detection and M3
guards):

- **W1 — `Scheduler` held across a swap.** `Scheduler::run` is
  `loop { services().scheduler.borrow_mut().step(); }`: one `&mut Scheduler`
  is live for the whole `step()`, including inside `switch_to_task`'s swap.
  While the scheduler thread runs with IF=1 (reconcile/start_next_task
  between swaps), a tick's `preempt()` borrows the same cell and mutates
  `MlfqStrategy` — a second live `&mut Scheduler`; `apply_pending_preemption`
  can even re-enter `switch_to_scheduler` there.
- **W2 — wake sequence interruptible mid-mutation.** Kernel-task
  `wake_tasks` runs at IF=1 holding `&mut scheduler` + `&mut task_manager`
  across `set_state(Ready)` → `push_ready`. A tick can preempt at any
  instruction there; if `preempt` decides to switch, kstatsd is suspended
  *mid-run-queue mutation* and the scheduler thread's `step()` mutates the
  same MLFQ state while the suspended frame is halfway through it.
- **W3 — split notify chain.** `notify_waiters` holds `&mut` into
  `MailboxManager.waiters` while calling `complete_ipc_message` (borrows
  `future_registry`) and `notify`; suspend anywhere in that chain (tick, or
  keyboard IRQ `deliver_to_service` on a mailbox task too) lets other
  mutators touch both cells before the suspended frame continues with live
  references into them.
- **W4 — post-swap consume.** A resumed parked task runs
  `FutureRegistry::consume` at IF=1 (after `sti; ret`); a tick suspending it
  mid-arena-mutation leaves a live `&mut future_registry` in the suspended
  frame while the next task's IPC syscalls mutate the same arena.
- **W5 — `ExecutionState` epilogue race.** Between `swap_context`'s `sti` and
  the epilogue's `self.preemption_enabled = false`, a tick sees
  `preemption_enabled == true` with `current_task` already loaded, and
  `apply_pending_preemption` → `switch_to_scheduler` can take that
  just-resumed handle and switch away again mid-handoff.

Reading of the set: every window needs only (a) an IRQ landing on an
IF=1 kernel-context instruction while an outer `&mut` of a cell it touches is
live. That is the precise contract M2's detector enforces and M3's `IrqGuard`
closes.

### M2 — Instrumentation; prove the lost wake — DONE 2026-11-01

Probe: `kernel/src/wake_probe.rs` (temporary). Guards on three cells
(`scheduler`, `future_registry`, `mailbox_manager`) with depth counting;
`REENTER` fires when an IRQ-context entry (IF=0) lands on a cell whose live
holder entered at IF=1. Lifecycle counters `park/notify/wake/pushed/picked/
swapped-in` tagged with task name + IF.

Rounds (`/tmp/opencode/m2/round-*.log`): 3/3 FAIL, all with REENTER firing
before the hang:

| round | REENTER scheduler | REENTER mailbox | REENTER future_registry | hang point |
|---|---|---|---|---|
| 1 | 47 | 8 | 0 | after `[ATAD] ATA disk driver`, drive-ready line never printed |
| 2 | 48 | 4 | 0 | `ROSE Shell` banner drawn, `rose>` prompt never appeared |
| 3 | 42 | 6 | 0 | same as round 1; last probe = `park [K] Kstatsd if=1` |

Verdicts:

1. **W1 + W3 are real and frequent.** The `tick → should_preempt` overlap on
   an IF=1-held `scheduler` cell (W1/W2) and `pop_front_async` (IF=0, inside
   a user `int 0x80`) landing on an IF=1-held `mailbox_manager` cell (W3)
   both occur tens of times per boot. The guards only print — the
   interleavings they expose exist identically without the probe; the probe's
   extra prints merely widen the windows and move the victim earlier
   (boot-time hang instead of the M0 `mem` hang; the failing victim varies
   round to round — classic corruption spread).
2. **Kernel-task park runs at IF=1.** `park task=[K] Kstatsd fh=NN if=1` is
   the last probe event in rounds 1 and 3: the kernel-task park sequence
   (`is_future_completed` → `block` → `register_waiter` → switch) is NOT
   atomic — a delivery completing the future between check and registration
   loses the wake (suspect 2's composite window confirmed to be open; the
   per-call FR guards do not cover the composite).
3. **`future_registry` shows zero IRQ overlap** — the corruption surface is
   `scheduler` + `mailbox_manager`, not the registry's own cells.

Note for M3: the probe's REENTER budget uses `fetch_sub` on an unsigned
counter (wraps to huge after exhaustion) — the production guard must use
`saturating_sub`.

Gate: PASSED — instrumented runs explain mechanically why wakes get lost:
IRQ/syscall re-entry mutates cells whose borrows are live from IF=1 contexts
(W1/W2/W3), and the kernel-task park composite is non-atomic.

Probe stays in-tree until M3 fixes are verified (REENTER lines should
disappear when the windows close); removed in M4.

### M3 — Fix, one change at a time

Ordered by likely evidence; stop when M2's verdict is addressed:

#### M3.1 — IF-boundary rework (DONE 2026-11-01, gates below)

M2 showed every proven window needs an IF=1 *kernel* context as the holder.
M3.1 removes that category entirely: no kernel-mode code runs with IF=1
except the idle `sti`/`hlt` loop.

Changes (both arches — M1 established the contract is shared):

- `swap_context` no longer ends with `sti; ret` (`arch/x86_32/src/cpu.rs`,
  `arch/x86_64/src/context_switching.S`): a resumed context continues at
  IF=0, so all post-swap kernel bookkeeping runs masked.
- x86_32 `int80_handler` forces IF=1 in the saved eflags before `iretd`
  (`or dword ptr [esp + 8], 0x200`) — the x86_32 counterpart of the
  existing `or r11, 0x200` in x86_64 `syscall.S`. Without this the forced
  IF=0 from `swap_context` would poison the caller's eflags snapshot and
  no user task could ever be preempted again.
- `idle_entry` re-enables before every `halt` (`kernel/src/task.rs`): the
  kernel-mode interrupt boundary.
- Consequences: kernel-task service loops (kstatsd…) run their short
  bodies at IF=0 and park on futures — cooperative in kernel mode, exactly
  the microkernel shape; user-mode preemption is unchanged (ticks land on
  IF=1 user code, gate masks, handoff runs masked). Cold-start first slice
  of a fresh task runs masked until its first syscall returns — bounded,
  every app syscalls immediately.
- Host-test safety of the (temporary) probe: `ifflag()` stubs to 0 under
  `cfg(test)`; probe prints compile to nothing under `cfg(test)`.

Verification:

- x86_32 debug `kstats_mem` **6/6 PASS** with the M2 probe still mounted:
  **zero `REENTER`** lines and zero `park ... if=1` — consistent with the
  design: `holder_if == 1` is now impossible, so the W1–W4 family is
  structurally dead, not merely rare.
- Host unit tests: 357/357.
- x86_64 full integration suite: **20/20 result blocks ok, exit 0**
  (`/tmp/opencode/m3x64/suite-final.log`).
- Artifact found during the x86_64 gate: with probe *prints* active,
  `shell_echoes_keystrokes` failed 2/2 (the probe's per-scancode
  `kprintln` from IRQ context contends with the console path). Bisected:
  same failure vanishes (3/3 + in-suite pass) with prints muted under
  identical M3.1 code — pure instrumentation artifact, no regression.
  Probe device-side prints are now muted; this is also a design note for
  `IrqGuard`: production path must be silent (trip once, loudly, and
  rarely).

Remaining M3 steps (2–5) now become defense-in-depth: the `IrqGuard`
primitive keeps the detector as a production tripwire for regressions of
this class, and the guarded structures no longer depend on tribal
IF-discipline.

1. **IF discipline (arch):** remove `sti` from `swap_context` on x86_32 (and
   x86_64 if M1 shows it is needed for correctness there too); re-enable
   interrupts only at the true boundaries — int80 `iretd` (already restores
   user IF via the saved eflags), the syscall-return path, the idle loop
   before `hlt`, and the scheduler-thread top loop. Kernel bookkeeping after
   a resume stays IF=0. Verify: debug `kstats_mem` 6/6.
2. **`IrqGuard` primitive:** new kernel module (~50 lines, built on the
   `Cpu` trait — stays platform-agnostic per the HAL rule): save-and-disable
   on acquire, restore on `Drop`, per-structure held-flag re-entry detection
   that fails loudly. The M2 detector survives as this guard's core; it also
   formalizes the pattern `memory_manager.rs` already hand-rolls. Its unit
   tests live in the module's `#[cfg(test)]` block; the production callers are
   steps 3–4.
3. **Guard the IRQ-reachable structures M2 proved re-entrant** (candidates:
   scheduler algorithm + `ExecutionState`, `FutureRegistry::waiters`,
   `MailboxManager`), replacing their ad-hoc masking convention with the
   guard.
4. **Notify atomicity:** make `push_back` → `notify_waiters` →
   `complete_ipc_message` → `notify` → `wake_tasks` → `push_ready` one
   `IrqGuard` section, closing the split-borrow window at
   `mailbox_manager.rs:93-109` for both kernel-task and IRQ senders.
5. Any residual corruption the guard still flags gets its own smallest fix,
   discussed and approved before coding.

Each numbered fix is its own commit and must keep the x86_64 suite green.

### M4 — Verification and unguard

1. Remove the M2 counters and trace prints. `IrqGuard` stays: its
   held-flag detector is production code with real callers (M3.2–4) per the
   no-test-only-surface rule.
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
