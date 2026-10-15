---

# Plan: Fix game triple-fault (snake/tetris idle) — turn `app_lifecycle` green

**Branch:** `fix/game-triple-fault` = `main(7171686)` + `97be6b7` (red tests + `QemuSession::is_running()`). Kernel code at `7171686`.
**Goal:** `tests-integration/tests/app_lifecycle.rs` (`tetris_survives_idle`, `snake_survives_idle`) PASS with their `#[ignore]` attributes removed.
**Author:** Architect subagent, 2026-10-11. No code in this document; contracts only.

## 1. Locked decisions (verbatim from user + orchestrator)

- **D1:** Fix at root cause in the scheduler/task lifecycle. Required core changes: MLFQ `push_ready` dedups (no duplicate handle across any queue); `on_task_terminate` removes ALL occurrences of the handle from every queue (loop until none found) + yield_info; comprehensive new unit tests in mlfq_strategy.rs for dedup, remove-all, pick_next-after-terminate safety (repo rule: unit tests for all kernel changes, see existing mlfq tests style).
- **D2:** Kernel is sole invariant authority: silent zero-tolerance in the switch path is forbidden. `switch_to_task`/context switch must never restore a context for a handle the TaskManager cannot resolve to a live task: prefer defensive skip (return/skip so the scheduler re-picks or falls to idle) plus debug_assert! to make bugs loud in dev. Changing `get_task_stack_stack_pointer` to return Option<usize> is acceptable if the blast radius stays within kernel crate call sites (enumerate call sites in the plan). NO println! spam in hot paths, NO comments anywhere (AGENTS.md). *(Note: decision text as delegated reads `get_task_stack_pointer`.)*
- **D3:** KernelCell re-entrancy: ONLY if D1+D2 do not turn the tests green, add the minimal diagnostic: a borrow-depth flag inside KernelCell making re-entrant borrow_mut panic loudly in dev instead of aliasing UB. Full HAL/redesign of the ISR/queueing architecture is OUT of scope (future architecture discussion).
- **D4:** The pre-fault FREEZE (lost wakeup: app blocked on a future that never completes, or double-queue livelock) must be diagnosed during implementation; D1 is hypothesized to eliminate it (double-queued task whose requeue_after_run removes the wake's yield_info entry etc.). If the freeze persists after D1+D2 with unit tests green, the coder must STOP and report evidence (frozen screen + which future/waiter) rather than broaden scope.
- **D5:** Kernel/ stays no_std, portable (no arch-specific code in kernel/), no comments, no new deps, minimal diff. Keep kernel/src/scheduler/scheduler.rs step() structure unchanged unless evidence demands otherwise.

## 2. Background

Launching snake/tetris from the shell on the **debug** kernel renders some frames, then freezes; ~13.5 s after spawn the guest triple-faults: `#PF` at RIP=0 (CPL=0, GPRs zero, stack below RSP = zero page) → `#DF` → triple fault. Forensics (2026-10-11, trusted): the context switched INTO was zeroed (`RIP=0`, `RSP=0x20000013b90` into zeroed fresh memory, deterministic across runs); the IRQ frame at the last tick sat on an ELF app task's stack (`elf_task_wrapper` at bottom, `syscall_handler_entry+0x40` trampoline; symbols `timer_interrupt_handler → Kernel::preempt → Scheduler::should_preempt → KernelCell::borrow`).

Code-level chain on `main`, re-verified by reading (all file:line below are current on this branch):

1. `kernel/src/scheduler/mlfq_strategy.rs:74-76` — `fn push_ready(&mut self, handle: TaskHandle) { self.queues[0].push_back(handle); }` — no dedup.
   `kernel/src/scheduler/mlfq_strategy.rs:87-94` — `on_task_terminate` removes only the FIRST matching occurrence per queue (`if let Some(pos) = queue.iter().position(…) { queue.remove(pos); }` once) — duplicates/stale handles survive removal; then `self.yield_info.remove(&handle);`.
   `requeue_after_run` (mlfq_strategy.rs:65-72) is the second enqueue edge and also pushes unguarded.
2. `kernel/src/task_manager.rs:69-74` — `pub(crate) fn get_task_stack_pointer(&self, handle: TaskHandle) -> usize { … Err(_) => 0, }` returns **0** for dead handles; `get_task_stack_pointer_ref` (lines 76-81) returns `null_mut()`; `set_state` (50-54) silently ignores dead handles; `get_state` (43-48) maps dead → `Terminated`.
3. `kernel/src/state.rs:22-40` — `switch_to_task` feeds that `0` straight into `self.cpu.swap_context(state_pointer, scheduler_stack_pointer_pointer, task_stack_pointer)` (line 35) — unconditional restore of a zeroed context → #PF → #DF → TF.
4. Re-entrant aliasing (believed cause of the pre-fault freeze/lost wakeups): `kernel/src/scheduler/mod.rs:42-46` `run()` = `loop { services().scheduler.borrow_mut().step(); }` and `step()`'s `switch_to_task` runs whole task executions **while that `&mut Scheduler` is live**; the timer ISR `kernel().preempt()` (`arch/x86_64/src/interrupts.rs:104-113`) and keyboard ISR `kernel().enqueue(...)` (`arch/x86_64/src/interrupts.rs:116-127`) re-enter the same cell — `kernel/src/kernel_cell.rs:23-26` is a bare `UnsafeCell` borrow with **no** tracking. Same pattern via `terminate_and_yield` → `notify` → wake → `SchedulerWakerUseCase::wake_tasks` (`kernel/src/kernel_services.rs:25-27`) from task context.

Production scheduler on x86_64 is MLFQ (`arch/x86_64/src/main.rs:43`: `scheduler_factory: scheduler::mfq_scheduler`), installed over the constructor-default FIFO cell at `kernel/src/kernel.rs:131-132` (`services().scheduler.replace(*scheduler);`).

Idle-state note (for D4): snake after GAME OVER (self-inflicted wall crash without input, `apps/snake/src/main.rs:181-191`) blocks in `loop { Syscall::read_char() … }` (`apps/snake/src/main.rs:224-234`) → `SyscallNum::ReadChar` registers a `KeyboardFuture` waiter and blocks (`kernel/src/syscall.rs:36-48`). During the 35/45 s idle window nothing terminates and no keys arrive — the guest must stay alive purely on ticks + shell + random_gen_server.

## 3. Baseline facts (sourced; do not re-run expensive baselines)

- Delegation baseline, established 2026-10-11 on this lineage: `boot_and_shell` **PASS (4 passed)**; `ipc_random` **PASS (1 passed)**; `app_lifecycle` **RED for the right reason** (dead guest ~13.5 s); unit suite green; repro script ran today for `snake` (`timeout`/QEMU/python3 tooling therefore confirmed working in this environment).
- Repro script exists and was read today: `/tmp/opencode/repro_triple_fault.py` (153 lines, outside the repo). It builds the image via `arch/x86_64-runner/target/debug/runner` (binary name `runner` confirmed in `arch/x86_64-runner/Cargo.toml`, standalone workspace), runs QEMU with `-action shutdown=pause -d int,cpu_reset -D …/exc.log`, types `<app>` + Enter at `rose>`, scripts `PHASES`, and prints machine-countable lines including `QEMU returncode:`, markers (`game_over_snake:`, `prompt_seen:`), and `exc.log: lines: N resets: M`.
- `get_task_stack_pointer` call sites **exactly** (grep-verified; blast radius is kernel-crate-only): `kernel/src/task_manager.rs:69` (definition), `kernel/src/kernel.rs:172` (`Kernel::start`, scheduler-task handle, added + `prepare_entry`'d in `Kernel::new` at kernel.rs:118-129 — always live there), `kernel/src/state.rs:26` (`switch_to_task`), `kernel/src/state.rs:54` (`switch_to_scheduler`, `self.scheduler` — never terminated). `get_task_stack_pointer_ref` call sites (unchanged): `state.rs:31`, `state.rs:48`.
- `cargo test -p tests-integration …` is valid: `tests-integration` is a workspace member (`Cargo.toml:18`); the integration harness builds the **dev** kernel itself to `target/rosx/debug/rosx` (`tests-integration/src/kernel_build.rs:24-76`) — same artifact the repro script consumes.
- xtask surface (read from `xtask/src/main.rs` + `.github/workflows/ci.yml`): `cargo xtask apps`, `cargo xtask build`, `cargo xtask build --debug`, `cargo xtask test --skip-integration` all exist; CI's unit gate is `cargo xtask test --skip-integration` (ci.yml:31).
- `cargo test -p kernel -- --test-threads=1` is the repo's kernel unit invocation (AGENTS.md); kernel unit tests share static `KernelServices` (`kernel/src/kernel_services.rs:77-123`) so single-threaded is mandatory.
- Command-green provenance: all acceptance commands below are either (a) proven green today by the delegation baseline, or (b) CI/AGENTS-canonical. The one command string without a same-day literal run is `cargo xtask build --debug` (its today-equivalents ran: `cargo xtask build` in CI form and the harness's dev-profile kernel build that produced a bootable guest at 13.5 s) — marked **unverified-literal**; orchestrator baseline pass covers it; never treat it as binding until proven.

## 4. Steps

### Step 1 — MLFQ queue hygiene (dedup + remove-all). File: `kernel/src/scheduler/mlfq_strategy.rs`

**What to change** (contract, no code):
- Add a private helper on `MlfqStrategy`, e.g. signature `fn contains_handle(&self, handle: TaskHandle) -> bool` — prose: returns true iff `handle` occurs in any of `self.queues` (iter over all `NUM_QUEUES` deques, value-compare `TaskHandle` which is `Copy + PartialEq` via `Handle`).
- `push_ready` (currently line 74): return without pushing when `contains_handle(handle)` is true; otherwise `push_back` on queue 0 as today. This makes the invariant *"no two equal handles across any queues"* inductive for this edge regardless of upstream double-wake source (wake_tasks `kernel/src/scheduler/scheduler.rs:140-147` can push a handle that is already Ready-queued or even Running; `Kernel::push_task` `scheduler.rs:133-138` can re-push a fresh Ready task — the strategy-level guard closes every enumerated edge at once, per D1).
- `requeue_after_run` (lines 65-72): consume `yield_info` exactly as today (`remove`), then **skip the `push_back` iff `contains_handle(handle)` is already true after the remove** (the double-queued-during-run case: wake pushed while the task was Running, then reconcile's `requeue_after_run` — `scheduler.rs:189-198` — would push a second copy). Skipping preserves queue position; yielding demotion info is consumed either way, so no stale `yield_info` leaks into a later requeue. Without this second guard the D1 invariant is not enforced on all edges of the same type.
- `on_task_terminate` (lines 87-94): for **each** queue, loop `while let Some(pos) = queue.iter().position(…)` and `queue.remove(pos)` until none found (termination: each iteration strictly shrinks the deque), then remove `yield_info` as today. This closes the exact forensic fault mechanism: with remove-once, the surviving stale copy is later `pick_next`'d after `remove_task`, `get_task_stack_pointer` yields 0 → `swap_context` to zero page → #PF→#DF→TF.
- No new imports; no comments; `yield_info` keying unchanged.

**New unit tests** (append to the existing `mod tests`, matching existing style: `make_handle(index, generation)` helper, direct field assertions on `strategy.queues[…]`):
- `push_ready_is_idempotent_for_same_handle` — push twice → `queues[0].len() == 1`, total across all queues == 1.
- `push_ready_skips_handle_already_queued_in_lower_queue` — push directly into `queues[2]`, then `push_ready` → `queues[0].len() == 0`, `queues[2].len() == 1`.
- `requeue_after_run_does_not_duplicate_already_queued_handle` — push `h`; `record_yield(h, Preempted)`; `requeue_after_run(h)` (h still queued) → total occurrences 1, `yield_info` no longer contains `h`.
- `on_task_terminate_removes_every_duplicate_occurrence` — push `h` twice (via `push_ready` and one direct `queues[1].push_back` to bypass the dedup guard and simulate legacy/corrupt state), plus `record_yield`; `on_task_terminate(h)` → no queue contains `h`; `yield_info` cleared; `pick_next().is_none()`.
- `pick_next_after_terminate_never_returns_dead_handle` — `h1`, `h2` queued; terminate `h1`; remaining picks yield only `h2`, then `None`.

**Verification gate:** `cargo test -p kernel -- --test-threads=1` → all pass, including all pre-existing scheduler tests (verified by reading: no existing test in `mlfq_strategy.rs`, `fifo_strategy.rs`, `scheduler.rs`, `mod.rs` enqueues the same handle twice through the strategy; direct `queues[i].push_back` test fixtures bypass guards and stay valid).

### Step 2 — Loud switch path: no silent zero restore (D2). Files: `kernel/src/task_manager.rs`, `kernel/src/state.rs`, `kernel/src/kernel.rs`

Must be one compilable unit (signature change + all call sites simultaneously).

**`task_manager.rs`:** change `pub(crate) fn get_task_stack_pointer(&self, handle: TaskHandle) -> usize` (line 69) to `-> Option<usize>`: `Ok(task) => Some(task.stack_pointer())`, `Err(_) => None`. Do **not** change `get_task_stack_pointer_ref` (still `*mut usize`/null for dead; its two call sites are save-direction on handles that are live by construction: `self.scheduler` and `current_task` — note in commit message). Keep `use core::ptr::null_mut;` (still needed by `_ref`). No behavioral change to `set_state`/`get_state`.

**`state.rs` `switch_to_task` (lines 22-40):** before any mutation of `self`, resolve the stack pointer via the new `Option` and apply a single defensive-skip predicate: `None`, `Some(0)`, — the contract is *"cannot resolve to a live, entry-prepared task"*. On skip: fire `debug_assert!` with a message naming the failure (fires in the debug profile, which is exactly what the integration tests run → `handle_panic` prints `!!! PANIC !!!` + file:line to the debug console — loud, per D2), then **return `task_handle` unchanged without touching `current_task`, `execution_context`, or `preemption_enabled`, and without calling `swap_context`**. Rationale for skip-and-return-handle: `KernelContextSwitcher::switch_to_task` (`kernel/src/kernel.rs:36-47`) maps the returned handle's state to `SwitchOutcome::Terminated` (`kernel.rs:44`), so `reconcile_returned_task` → `handle_termination` (`scheduler.rs:166-172`) drops any residual copies via the now-remove-all `on_task_terminate` and re-picks/falls to idle on the next `step()` — self-healing, no println, no panic in release. On the normal path, behavior is byte-for-byte as today plus unwrap of the `Option` (use a `match`/`let … else` in prose form; do not introduce imports).
**`state.rs` line 54 (`switch_to_scheduler`)** and **`kernel.rs:172` (`Kernel::start`)**: scheduler/main-thread handles are added and `prepare_entry`'d before use (`kernel.rs:118-129`, `kernel.rs:152-163`) — resolve the `Option` with `expect` carrying a static message string.

**New unit tests** (`task_manager.rs`, extend existing `mod tests`): after `add_task`, `get_task_stack_pointer(handle)` is `Some(_)` and non-zero (`Task::new` at `kernel/src/task.rs:66-83` pre-sets SP, so no `Cpu` needed); after `remove_task(handle)`, it is `None` and `get_state(handle) == Terminated`.

**Verification gates:**
- `grep -E 'pub\(crate\) fn get_task_stack_pointer\(&self, handle: TaskHandle\) -> Option<usize>' kernel/src/task_manager.rs` → exactly 1 line; `grep -rn -E 'get_task_stack_pointer' kernel/src` → the 7 enumerated sites from §3 only, no others.
- `cargo test -p kernel -- --test-threads=1` → green.
- `cargo xtask build --debug` → green (**unverified-literal** per §3; if it fails on an environment issue, report, do not paper over). Note: this build carries `debug_assertions` ON — a firing assert on QEMU is a *finding* (red + evidence), never to be silenced.

### Step 3 — Wake/enqueue call-site audit (mostly read-only; smallest correct guard only on evidence). Files: audit all; possible touch: `kernel/src/scheduler/scheduler.rs`

Enumerated enqueue edges and why Step 1's strategy-level dedup closes each concrete window:
- `wake_tasks` `scheduler.rs:140-147` (reachable via `SchedulerWakerUseCase` `kernel_services.rs:25-27` ← `FutureRegistry::notify` `future.rs:145-148` ← timer `kernel_services.rs:56-63`, keyboard `keyboard.rs:20-28` (gated by `was_empty`), task-completion `kernel.rs:270`, mailbox `mailbox_manager.rs:90-107`): can target a handle that is Ready-and-queued or Running (single-CPU interleavings plus the KernelCell aliasing UB can make the `get_state != Terminated` check at `scheduler.rs:142` act on torn state). Window closed at the queue edge by dedup.
- `reconcile_returned_task` Yielded branch `scheduler.rs:191-198` (`record_yield` + `requeue_after_run`) racing a wake push during the task's own run — closed by the requeue guard in Step 1 (the exact double-push the delegation hypothesizes).
- `push_task` `scheduler.rs:133-138` ← `schedule_task` `kernel.rs:295`: fresh Ready task pushed once; re-push idempotent via dedup.
- Repeated `notify` on one future: already idempotent (waiters are `remove`d, `future.rs:146`; pinned by test `notify_does_not_double_wake`, `future.rs:202-219`).
- `wake_tasks`' Terminated filter (`scheduler.rs:142`) must remain (pinned by `wake_tasks_should_not_resurrect_terminated_task`, `scheduler.rs:733-747`).

**Deliverable:** the audit either records (in the commit message) "no further code guard: strategy-level dedup covers every enumerated edge", or — **only if** the repro in Step 5 shows state-clobbering symptoms from waking a `Running` task (symptom signature: task Ready but not in any queue, `SwitchOutcome::Unchanged` at `kernel.rs:45` for a task that was running) — adds exactly one guard in `wake_tasks`: skip both the state overwrite and the push when `get_state(handle) == TaskState::Running`, plus a unit test on `Scheduler` using the existing `FakeTaskManager`/`FakeAlgorithm` harness. Do not add speculative filters beyond this (D5). Keep `step()` structure untouched (D5).

**Gate:** `cargo test -p kernel -- --test-threads=1` green (no-op if audit yields no code change).

### Step 4 — Profile + apps build matrix. Files: none (commands only)

- `cargo xtask apps` → green (user-ELF set unchanged; shell embeds `target/rosx-user/release/{snake,tetris,…}` via `apps/shell/build.rs` — must exist before Step 6).
- `cargo xtask build` → green (release; also refreshes `arch/x86_64-runner/target/debug/runner` needed by Step 5).
- `cargo xtask build --debug` → green; produces `target/rosx/debug/rosx` (the profile the tests and repro run). **unverified-literal** per §3.

### Step 5 — Fast runtime loop (before flipping `#[ignore]`). Files: none

Run for **both** games, in this order, from the repo root:

```
KERNEL=/home/ghisi/git/rosx/target/rosx/debug/rosx PHASES="wait:35" timeout 70s python3 /tmp/opencode/repro_triple_fault.py snake
KERNEL=/home/ghisi/git/rosx/target/rosx/debug/rosx PHASES="wait:35" timeout 70s python3 /tmp/opencode/repro_triple_fault.py tetris
```

(Precondition: Step 4 built the debug kernel; the script rebuilds the disk image itself via `arch/x86_64-runner/target/debug/runner`. Script presence + mechanics verified today; harness tooling confirmed by the delegation's same-day run.)

**GREEN contract (machine-countable):** printed line `exc.log: lines: N resets: 0` (no `cpu_reset` = no triple fault within the 35 s horizon), guest alive through the phase (no early QEMU exit), snake additionally shows `game_over_snake: True` with the screen tail still rendering frames; tetris shows continued frame output in the stdout tail. Any `resets: ≥ 1` or a dead returncode = still red.

**STOP-reporting rule (D4, binding):** if unit tests (Steps 1-3) are green and unit gates pass but the repro still shows the **freeze** (guest alive but repaints stop — lost wakeup / double-queue livelock) or any reset, the coder **stops and reports evidence**: the screen tail, the `exc.log` tail, the probe output (`PROBE_EXTRA`/`info registers` hooks exist in the script) identifying which future/waiter the game task is parked on, and the exact command lines run. Do NOT broaden scope; do NOT add D3 code without orchestrator go-ahead.

### Step 6 — Un-ignore the red tests + full verification matrix. File: `tests-integration/tests/app_lifecycle.rs`

Remove the two attributes verbatim in full (both start with `#[ignore = "red: deterministic triple fault after launching`):
- line 16: `#[ignore = "red: deterministic triple fault after launching tetris with no further input; see 2026-10-11 investigation"]`
- line 58: `#[ignore = "red: deterministic triple fault after launching snake with no further input; see 2026-10-11 investigation"]`

Leave the test bodies and all timing constants untouched.

**Gates (expensive; run once, in this order):**
1. Green gate: `cargo test -p tests-integration --test app_lifecycle -- --ignored --test-threads=1` → **2 passed, 0 failed** (~4 min; two QEMU boots).
2. `cargo test -p tests-integration --test boot_and_shell -- --test-threads=1` → **4 passed** (baseline parity).
3. `cargo test -p tests-integration --test ipc_random -- --test-threads=1` → **1 passed** (baseline parity).
4. `cargo xtask test --skip-integration` → green (full unit suite, CI-equivalent).
5. `grep -E '#\[ignore' tests-integration/tests/app_lifecycle.rs` → no output (exit status 1).

### Step 7 — ONLY if Steps 5/6 still fault after D1+D2 with unit tests green (D3 diagnostic, scratch only)

Do this on a **scratch commit/branch**, never as the committed green state:
- `kernel/src/kernel_cell.rs`: add `borrow_depth: core::cell::Cell<u8>` alongside `data` (`Cell::new(0)` keeps `KernelCell::new` a `const fn`); change `borrow_mut` to return a `pub(crate)` guard type implementing `Deref`/`DerefMut<Target = T>` with `Drop` decrementing; increment in `borrow_mut` and `debug_assert_eq!(depth_before, 0, "re-entrant KernelCell::borrow_mut")` **only under `debug_assertions`**. `borrow`/`replace` unchanged.
- Contract warning the coder must verify before interpreting results: legitimate re-entrancy **exists today by construction** — the outer `services().scheduler.borrow_mut().step()` (`scheduler/mod.rs:44`) is live for the whole task execution, so a timer tick's `preempt()` → `scheduler.borrow_mut().should_preempt()` (`kernel.rs:248-255`) enters the same cell during any preempted quantum. A committed unconditional panic would halt the guest on the first preempt even in a fixed kernel. Therefore: run scratch, capture **which** re-entry fires first (timer tick mid-step vs. `terminate_and_yield` → notify wake), report the evidence for the architecture discussion (out of scope per D3), and revert. If the scratch run's recorded sites are all known-benign-looking ticks, that confirms the aliasing UB as the remaining root cause and the fix belongs to a future ISR/HAL design change — an orchestrator decision, not this branch's.

## 5. Blast radius (tests that transitively reach the changed code)

- **Global service registry reach:** `kernel/src/kernel_services.rs` statics + `wire_scheduler_for_tests` (`scheduler/mod.rs:49-64`) wire `FutureRegistry`'s wake controller to the global scheduler cell; therefore `kernel/src/future.rs` tests (`notify_moves_waiters_to_ready`, `notify_does_not_double_wake`), `kernel/src/scheduler/mod.rs` tests (`fifo_scheduler_factory_produces_functional_scheduler`, `mfq_scheduler_factory_produces_functional_scheduler` — real `MlfqStrategy` wake at mod.rs:110), and `mailbox_manager.rs` tests (`test_push_back_notifies_waiters`) transitively execute the Step 1 strategy. All perform single pushes → unaffected; they must nonetheless run green under `--test-threads=1`.
- **Constructor replacement plumbing:** `Kernel::new` (kernel.rs:131-132) replaces the constructor-default FIFO scheduler cell with the MLFQ one — Step 1 changes behavior only for the production (MLFQ) path; `FifoStrategy` is deliberately left un-deduplicated (minimal diff, D5; known deferred gap, call it out in the commit message).
- **`scheduler.rs` `FakeAlgorithm` tests** — trait `SchedulingAlgorithm` signatures unchanged → compile + green invariant.
- **Guest-side (no host coverage):** `state.rs` switch logic and the Step 2 `debug_assert` execute only in QEMU — proven by the repro (Step 5) and all three integration binaries (Step 6), not by unit tests.
- **`tests-integration` crate itself** (std, host) is unaffected by kernel edits; only guest behavior changes its outcome.

## 6. Risks & edge cases

- **`debug_assert` fires during Step 5/6 dev runs** → guest prints `!!! PANIC !!!` (`kernel/src/panic.rs:4-17`) and halts without triple-faulting; `is_running()` may stay true → the repaint assertion message, not the triple-fault message, will be the failure text. Treat as red + evidence of a remaining stale-switch source; do not delete the assert.
- **Dedup scan cost:** bounded by `GenerationalArena<SharedTask, 256>` (`task_manager.rs:8`) × 3 queues; runs on wake/enqueue frequency, not on the tick cycle — acceptable for a dev OS.
- **`requeue_after_run` skip case** must still consume `yield_info` (contract in Step 1) or a stale priority entry misplaces a later requeue.
- **Skip path in `switch_to_task`** must not mutate `current_task`/`preemption_enabled`/`execution_context` and must not call `swap_context`; correctness depends on `KernelContextSwitcher`'s `Terminated` mapping (`kernel.rs:44`) making the scheduler self-heal.
- **`Kernel::start` / `switch_to_scheduler` `expect`s** are safe only because the scheduler task is never removed (`terminate_and_yield` is reached only from task wrappers); if a future change removes the scheduler task, they panic loudly — intended.
- **No comments / no new deps / no arch-specific code in `kernel/`** (AGENTS.md); existing comments in touched test files must not be propagated to new code.
- **D3 guard panic-on-benign-tick** (Step 7 warning) — the diagnostic is interpretable only against the known re-entrant sites enumerated there.
- **Integration flakiness:** each ignored test is a full QEMU boot (~2 min); run them sequentially (`--test-threads=1`), never in parallel with the repro (QEMU + temp-dir contention).

## 7. Acceptance criteria (exact commands)

Build/test commands that must pass, in order, on the final tree (Step 6 applied):

1. `cargo test -p kernel -- --test-threads=1` — all unit tests pass (includes the new mlfq + task_manager tests).
2. `cargo xtask apps` — green.
3. `cargo xtask build --debug` — green (**unverified-literal** until orchestrator baseline proves it verbatim).
4. `cargo xtask build` — green.
5. `cargo test -p tests-integration --test app_lifecycle -- --ignored --test-threads=1` — **`2 passed`**.
6. `cargo test -p tests-integration --test boot_and_shell -- --test-threads=1` — **`4 passed`** (baseline parity, established today).
7. `cargo test -p tests-integration --test ipc_random -- --test-threads=1` — **`1 passed`** (baseline parity, established today).
8. `cargo xtask test --skip-integration` — green.
9. `grep -E '#\[ignore' tests-integration/tests/app_lifecycle.rs` — zero matches.

Warning posture: no new warnings beyond the lineage baseline attributable to files touched here (Δ-new = 0 against the orchestrator's recorded baseline fingerprint; decreases at deleted sites are expected, not violations). No absolute-zero warning claim is made, and none may be substituted.

## 8. Commit-ready summary (final commit body; no prefix per AGENTS.md; keep Steps 1/2/(3)/6 as separate small commits with confirmations per AGENTS.md workflow)

```
Fix scheduler lifecycle to end the snake/tetris triple fault

MLFQ could hold duplicate handles (push_ready/requeue_after_run never
deduplicated) and on_task_terminate removed only one occurrence, so a
terminated task's stale handle stayed schedulable; switch_to_task then
restored the zeroed context of the freed arena slot: #PF at RIP=0 -> #DF
-> triple fault.

- push_ready and requeue_after_run skip handles already queued in any MLFQ queue
- on_task_terminate removes all occurrences of the handle and its yield info
- get_task_stack_pointer returns Option<usize>; switch_to_task defensively skips
  and debug_asserts instead of restoring a dead context, letting the scheduler
  re-pick or fall to idle
- new unit tests for dedup, remove-all, and pick_next-after-terminate
- enable tetris_survives_idle and snake_survives_idle
```

Deferred follow-ups to note (not this branch): FIFO strategy dedup gap; KernelCell ISR re-entrancy architecture discussion (D3 evidence, if gathered).

---

# Revision 2 (2026-10-11) — post-D4 pivot: switch-window mechanism

**Basis:** coder report `.opencode/artifacts/game-triple-fault-coder-report.md` (D4 STOP, Steps 1–4 green, fault byte-identical after D1+D2). Rev-1 Steps 1–3 stay **in the tree** (defense-in-depth per D1/D2); Step 6 remains gated. This revision supersedes Rev-1 §4 Step 5 GREEN wording (the `resets:` line is void — see §R2.2) and Rev-1 Step 7's panic-form diagnostic (superseded by a counter form).

## R2.0 Evidence ledger (sourced from the coder report transcript; do not re-run)

- Report row #1: `cargo test -p kernel -- --test-threads=1` → `test result: ok. 189 passed; 0 failed`. **Unit baseline is now 189** (182 + 7 new).
- Report rows #3/#4/#5: `cargo xtask apps`, `cargo xtask build`, `cargo xtask build --debug` all exit 0 today on the modified tree. This **retires Rev-1 §3 "unverified-literal"** on `cargo xtask build --debug`.
- Report row #2: `cargo xtask test` exit 0, incl. `boot_and_shell` 4 passed, `ipc_random` 1 passed, `app_lifecycle` 2 ignored.
- Report rows #7–#9: fault persists, deterministic signature `RIP=0, RSP=0x20000013b90` across apps/runs/pre/post-D1+D2; `panic: False` ⇒ the D2 `debug_assert!` in `switch_to_task` **never fired** ⇒ the bad restore does not flow through the `get_task_stack_pointer == None/Some(0)` path. H4 (dead-handle restore via the Option path) is REFUTED as the active vector; H5 (harness artifact) REFUTED.
- Report row #11: warning fingerprint Δ-new=0 (kernel lib 65↔65, rosx bin 26↔26, shell lib 5, +1 pre-existing manifest/future-incompat). This is the recorded baseline fingerprint for the warning posture below.
- Report row #13: D2 assert string present in the debug artifact (post-edit rebuild) — the negative at row #8 is real, not a stale-kernel artifact.
- Report §deviations: Rev-1 Step 5's `timeout 70s` binding form fails as a harness property (prompt written without trailing newline; script burns its 40 s prompt wait) — superseded by the 110 s floor + `-u` form in §R2.2.

## R2.1 Mechanism hypotheses (re-verified against the current tree)

- **H1 (top) — stale-flag IRQ window at the switch boundary.** `switch_to_task` sets `self.preemption_enabled = true;` (`kernel/src/state.rs:43`) *before* `self.cpu.swap_context(...)` (`:45`) and clears it only on return (`:46`), while asm ends `sti` (`arch/x86_64/src/context_switching.S:40`) then `ret` (`:41`). A tick landing between `sti` and the target's flag re-arm sees IF=1 with a stale `true` **while still in scheduler-side context**; the sole consumer `Kernel::preempt` (`kernel/src/kernel.rs:250`: `if self.execution_state.preemption_enabled && services().scheduler.borrow_mut().should_preempt() {`) can then run a phantom `switch_to_scheduler` that plants a one-frame-shifted context (fatal RSP `0x20000013b90` sits ~0x1b8 below the observed live idle halt frame `0x20000013d48`; task stacks are zero-initialized at `kernel/src/task.rs:73`, explaining the RIP=0/GPR-zero restore). Forensic anchor: snake's final tick `IP=0008:000000000022de47` = `arch/x86_64/src/cpu.rs:130` `swap_context` wrapper — the interrupt landed immediately after asm `sti; ret`, in kernel CS.
- **H2 (co-factor) — `KernelCell` re-entrancy aliasing UB** (`kernel/src/kernel_cell.rs:23-26` bare `UnsafeCell`; outer `services().scheduler.borrow_mut().step()` live across task execution at `kernel/src/scheduler/mod.rs:42-46`; ISR re-entry `preempt()`/`enqueue()` at `arch/x86_64/src/interrupts.rs`). D3 diagnostic is now **activated** (its "only if D1+D2 fail" condition is met) but stays scratch-only, never committed.
- **H3 — corrupted-but-nonzero saved SP** (variant of H1/H2; heap `Box<Task>` addresses are not nm-visible ⇒ attribution is best-effort).
- **H4 — stale/duplicate ready-queue handle: REFUTED as active vector** (D1+D2 green yet fault identical; D2 assert silent). The implemented hygiene stays as defense-in-depth.
- **H5 — harness artifact: REFUTED.**

**Fix-A preempt-preservation proof (verified by reading, this is what makes the one-line deletion shippable rather than symptom-masking):** every legitimate resume re-arms the flag independent of `state.rs:43` — mid-execution resumes return through `switch_to_scheduler`'s epilogue (`state.rs:69 self.preemption_enabled = true;`; all task-initiated yields route through it: `wait_future` `kernel.rs:229`, `task_yield` `:246`, `preempt` `:254`, `terminate_and_yield` `:275`), and first entries set it in the wrappers (`task.rs:168` `idle_entry`, `:187` `task_wrapper`, `:224` `elf_task_wrapper`). `state.rs:43` therefore only ever affects the `sti`→resume window; deleting it closes the window without disabling preemption on any legitimate path. ISR save/restore sites (`Kernel::schedule` `kernel.rs:182-183/:199`, `Kernel::enqueue` `:204-205/:207`) restore whatever the window holds and are re-armed by the same epilogues.

## R2.2 Corrected runtime GREEN contract (binding; supersedes Rev-1 Step 5 GREEN wording)

`resets:` is **void** — per coder report: "`resets: 0` in every run is a harness artifact: `-action shutdown=pause` pauses the VM on triple fault instead of logging `cpu_reset`; script then kills QEMU (`returncode: -9`)". Discriminate with greps against the script's printed `tmp:` dir, never with the `resets:` line.

Run sequentially (never two QEMU at once — temp-dir/contention), from the repo root, always `python3 -u`, timeout floor **110 s**:

```
KERNEL=/home/ghisi/git/rosx/target/rosx/debug/rosx PHASES="wait:35" timeout 110s python3 -u /tmp/opencode/repro_triple_fault.py snake 2>&1 | tee /tmp/opencode/repro_snake_g.log
KERNEL=/home/ghisi/git/rosx/target/rosx/debug/rosx PHASES="wait:35" timeout 110s python3 -u /tmp/opencode/repro_triple_fault.py tetris 2>&1 | tee /tmp/opencode/repro_tetris_g.log
```

Then, per app (POSIX):

```
TMP=$(grep '^tmp:' LOG | tail -n1 | awk '{print $2}')
grep -c -E 'Triple fault' "$TMP/exc.log"    # BINDING: must be 0
grep -c '!!! PANIC !!!' LOG                 # must be 0 (a PANIC banner = red + finding, never silence)
grep -c -E 'v=0e' "$TMP/exc.log"            # must be 0 (strongly expected)
grep -c -E 'v=08' "$TMP/exc.log"            # must be 0 (strongly expected)
```

Interpretation: returncode `-9` = script completed its teardown (normal; decide by greps). Returncode `124` = **inconclusive → rerun once with timeout +50 s**; never treat a truncated log as green/red. Continuation markers required: snake stdout tail still rendering board frames at the horizon; tetris likewise (`prompt_seen: True` alone is boot-only and insufficient).

## R2.3 Steps

### Rev-2 Step 1 — R1 forensic probe runs (no repo change)

Both apps, sequential, from the repo root:

```
KERNEL=/home/ghisi/git/rosx/target/rosx/debug/rosx PHASES="wait:8;probe;wait:12;probe;wait:40" PROBE_EXTRA=0x20000013a90,0x20000013d00 timeout 160s python3 -u /tmp/opencode/repro_triple_fault.py snake 2>&1 | tee /tmp/opencode/repro_snake_r1.log
```

(then the same for `tetris` → `/tmp/opencode/repro_tetris_r1.log`). Addresses straddle the fatal `0x20000013b90` and the live halt frame `0x20000013d48`; the script's `probe` phase (`/tmp/opencode/repro_triple_fault.py:102-120`) emits `info status`, `info registers`, `info symbol <RIP>`, `x/48gx <RSP>`, and `x/64gx <addr>` per `PROBE_EXTRA` value.

**Gate (evidence, not green):** each log contains the `--- PROBE status ---` sections and a `tmp:` line; the fault chain's last `v=20` tick IP before the final `v=0e`/`v=08` pair is recorded from `"$TMP/exc.log"`. Interpret new IPs only against the already-mapped anchors {`0x22de47` = `swap_context` wrapper `arch/x86_64/src/cpu.rs:130`; `0x22b994` = `syscall_handler_entry+0x40`; `0x287f80` = btree `first_edge` (scheduler-side drop)}; `nm`/`objdump` availability is **unverified** in this environment — do not gate on them. H1 predicts the memory at `0x20000013b90` looks like a swap-context save frame (register-save pattern + kernel-code return address) shifted from the live idle frame, not a live user frame.

### Rev-2 Step 2 — R3a causal single-lever scratch experiment (kernel-only, no USER-GATE)

**Exactly one edit:** delete line `self.preemption_enabled = true;` (`kernel/src/state.rs:43`) from `switch_to_task`. Nothing else — not `:46`, not asm, not wrappers. Save the tree diff first (`git diff > /tmp/opencode/rosx_pre_r3a.diff`) for exact restoration if red.

Verification sequence:
1. `cargo test -p kernel -- --test-threads=1` → `189 passed; 0 failed` (baseline sourced from report row #1; any count change is a finding).
2. `cargo xtask build --debug` → exit 0.
3. §R2.2 GREEN runs on both apps.

**Decision:** both apps `Triple fault` count 0 **and** `!!! PANIC !!!` 0 **and** continuation markers present ⇒ H1 confirmed; this one-line deletion **is** Fix-A → Rev-2 Step 4. Byte-identical fault ⇒ H1 window refuted as the carrier; restore from the saved diff and go to Rev-2 Step 3. PANIC banner ⇒ the D2 assert now fires — that is red + a new loud finding (Rev-1 §6 risk rule: never delete the assert); stop and report.

### Rev-2 Step 3 — R2 scratch KernelCell re-entrancy **counter** (scratch only, never committed; supersedes Rev-1 Step 7 panic form)

The panic form is known-useless: benign tick re-entrancy fires by construction (Rev-1 Step 7 contract warning; report row #8 `panic: False` shows nothing fatal was already asserted). Contract: in `kernel/src/kernel_cell.rs`, add a `core::cell::Cell<u8>` depth alongside `data` plus a high-water `Cell<u8>`; `borrow_mut` increments, records max, and returns a `pub(crate)` guard implementing `Deref`/`DerefMut<Target = T>` whose `Drop` decrements; `borrow`/`replace` unchanged; **no panics, ever**. Readout: one-shot `kprintln!` (the macro already used at `kernel/src/kernel.rs:309`) guarded by a `Cell<bool>` "announced" flag, printing the first depth≥2 observation. Gate: scratch snake run prints a depth≥2 observation naming which path (expected: tick-mid-step vs `terminate_and_yield`→notify-wake — attribution for the D3 architecture note). **Revert before any further gate**; final-tree check `grep -E 'Cell<u8>|announced' kernel/src/kernel_cell.rs` → zero matches.

### Rev-2 Step 4 — Promote Fix-A (only if Rev-2 Step 2 green)

Keep the same single-line deletion; it becomes the committed change (fmt-neutral: single line removed; coder report row #10 hunk-count method shows Δ-hunk 0 for a pure deletion). No USER-GATE (no `arch/` edit, no change to when interrupts are enabled — the asm `sti`/`cli` behavior is untouched; only the *preemption flag* stops being set early). Commit-message addendum to Rev-1 §8 body (one bullet): switch-window explanation + the R2.1 re-arm-site proof. Then re-run §R2.2 on both apps against the final tree, then Rev-2 Step 6.

### Rev-2 Step 5 — Fix-C fallback (ONLY if Step 2 red AND Step 3 attributes mid-window re-entrancy; **USER-GATE: requires explicit user approval before any edit**)

Contract at contract-level: remove `sti` (`arch/x86_64/src/context_switching.S:40`) so `swap_context` returns with the interrupt state it was given (`cli` at line 4 stays), and give IF explicit owners — kernel-side `self.cpu.enable_interrupts()` (call form at `kernel/src/kernel.rs:174`) at the same sites that re-arm the flag: `switch_to_scheduler` epilogue (after `state.rs:69`), `switch_to_task` scheduler-side epilogue (after `:46`), and the three wrapper entry points after their flag sets (`task.rs:168/:187/:224`). Before touching asm the coder must audit `initialize_stack` in `arch/x86_64/src/cpu.rs` for rFLAGS/IF semantics of constructed first-entry frames (the asm `ret` into them is not an `iretq`). Because Fix-C edits `kernel/src/state.rs` (shared, multi-arch), the x86_32 parity gate applies: build per AGENTS.md from `arch/x86_32/` (`cargo build -p rosx-x86`). All Fix-C edits + this section's rationale require recorded user approval; without it, STOP and report instead.

### Rev-2 Step 6 — Ship gate: un-`#[ignore]` + full matrix (unchanged from Rev-1 Step 6 / §7)

Only after Step 4 (or Step 5, if approved) is §R2.2-green on the final tree. Remove verbatim the two attributes (`tests-integration/tests/app_lifecycle.rs:16` tetris, `:58` snake, both `#[ignore = "red: deterministic triple fault after launching …"]`), bodies untouched, then run Rev-1 §7 gates 1–9 in order with baselines updated: gate 1 expects **189 passed** (report row #1); gates 6/7 baselines 4 passed / 1 passed (report row #2).

## R2.4 Blast radius additions

- **Fix-A touches flag read through the global services registry:** `preemption_enabled` lives on `ExecutionState`, reached by `Kernel::preempt` via the global cell; full site enumeration (grep-verified this turn, 21 repo hits): definition/`state.rs:15`; sets `task.rs:168/:187/:224`, `state.rs:43(deleted)/:46/:59/:69`, `kernel.rs:140/:175/:183/:199/:205/:207/:263`; read `kernel.rs:250`. No host unit test executes the swap path (Rev-1 §5), so guest proof = §R2.2 runs + all three integration binaries; host proof = the 189 suite.
- **Fix-C blast radius:** `arch/x86_64/src/context_switching.S` (shared by every task transition), `arch/x86_64/src/cpu.rs` (`initialize_stack`, `swap_context` wrapper), ISR assumptions in `arch/x86_64/src/interrupts.rs`, and — because epilogue edits land in shared `state.rs` — the **x86_32 kernel build**; add `cargo build -p rosx-x86` (run from `arch/x86_32/`) to that branch's matrix.
- Rev-2 Steps 2/3 are scratch: their blast radius is nil once reverted; gate `git status --porcelain` before the final matrix shows only the intended ship files (per coder report: 4 modified kernel files + `state.rs` line deletion when Fix-A ships; `plans/**` untracked is fine).

## R2.5 Stop rules (binding)

1. R3a red byte-identical → restore diff, run Rev-2 Step 3 for attribution, then **D4 stop-and-report** with: both R1 logs, the counter readout, exact command lines. No Fix-C without user approval.
2. Any `!!! PANIC !!!` banner → red + finding; the assert stays.
3. Freeze-with-live-guest (repaints stop) → D4 evidence rule (screen tail, exc.log tail, waiter attribution).
4. Scratch code (R2 counter, any probe hooks) never in a commit; Fix-C never without a recorded user-approval line in the orchestrator transcript.
5. One QEMU process at a time; `124` = inconclusive rerun (+50 s), never a verdict.

## R2.6 Acceptance criteria (final tree)

Rev-1 §7 items 1–9, with: item 1 baseline **189 passed** (report row #1); `cargo xtask build --debug` is now proven-run (report row #5) — provenance tag retired. Plus: §R2.2 greps green for both games on the final artifact, `grep -E 'Cell<u8>|announced' kernel/src/kernel_cell.rs` zero matches, and (Fix-C branch only) `cargo build -p rosx-x86` from `arch/x86_32/` green. Warning posture unchanged: **Δ-new = 0 against the report-row-#11 fingerprint** (kernel lib 65, rosx bin 26, shell lib 5, +1 manifest/future-incompat); any re-fingerprint filter must match both `warning:` and `warning[EXXXX]:` forms; decreases at legitimately deleted sites are expected and attributed by baseline multiset diff, never counted as violations.

# Revision 3 (2026-10-11) — mechanism re-rank + user decision package

## R3.0 Evidence re-read — corrections to the Rev-2 ledger

Verified this turn from `/tmp/opencode/repro_snake_r1.log` (and coder transcript for the tetris twin):

1. **All forensics so far are post-mortem.** The *first* probe section already prints `VM status: paused (shutdown)`. Timeline: the driver burns ≤40 s waiting for a `rose>` prompt (which the kernel never terminates with a newline), then sends the app keypresses, then `PHASES="wait:8;probe;…"`. Guest death therefore occurred **before the first probe (~t+48 s)**, i.e. within a few seconds of app spawn — consistent with the ~13 s-after-spawn estimate, and the rendered-board stdout proves spawn/gameplay preceded death. Consequence: R1's "region all zeros" remains valid as a **snapshot at time of death** (death does not zero memory), but **zero live-time samples exist**. Every timeline-based inference in Rev-2 that depended on "sampled while running" is void.
2. **Corpse state:** all GPRs `0x0`, `RIP=0`, `RSP=0x20000013b90`, `RFL=0x206` (IF set), region `0x20000013a90–0x20000013f00` all zeros except one word `0x0200000000000000` @ `0x20000013d98` (unattributed; note `0x13d98 − 0x13b90 = 0x208`, which corrects Rev-2's provisional `+0x50` arithmetic — leave unattributed).
3. **Frame-size math (new, from `cpu.rs`):** a saved/restored frame is exactly **16 words**: `initialize_stack` writes `entry_point` + 15 sentinel GPR words (`0x15` down to `0x0f`, plus `param1/param2` in the rdi/rsi slots); `context_switching.S` pops 15 GPRs and `ret`s into word 16. The corpse (15 zero GPRs, `RIP=0`, SP consistent with the final `ret`'s frame boundary) is exactly **one 16-word frame-pop over zeroed memory**. Sentinels would be `0x0f`–`0x15` if `initialize_stack` had written that slot; a legitimate save-side push would write real register values. Therefore **the fatal slot was never written, or was written and then zeroed** — and the only in-tree zero-writer of a 16 KiB stack region is `Task::new`'s zero-init (`kernel/src/task.rs:73`), which runs when a **freshly allocated** `Box<Task>` lands at that address.
4. **Cross-build invariance corrected:** the fatal **data** SP (`0x20000013b90`) is identical across both apps and across the R3a rebuild; the last-tick **code** IP is build-specific (`0x22de47` pre-R3a vs `0x28b870`/`0x288310` after). The invariance that matters is the data address ⇒ deterministic allocation-layout-dependent corruption, not race-timing coincidence. Rev-2's "identical tick anchor" phrasing overstated; retracted.
5. Standing facts unchanged: R3a RED byte-identical (H1 flag is not the carrier); KernelCell re-entrancy depth=2 proven, first occurrence at boot before any tick (one-shot readout consumed); tick-vs-terminate attribution unknown.

## R3.1 Mechanism re-rank

| Rank | Mechanism | Status against artifacts |
|---|---|---|
| **M1** | **Save-side slot/aliasing corruption.** `get_task_stack_pointer_ref` deliberately outlives its borrow (`kernel/src/state.rs:38-41`, `:55-58`); the raw `*mut usize` is written by asm `swap_context` during the open interrupt window. If a stale/aliased ref (recycled `Box<Task>` slot) receives the save, a live SP value lands in a dead/recycled box; the matching load later reads an SP whose 16-word slot was since **zero-initialized by `Task::new` at the same address** (`task.rs:73`). | **Leads.** Explains all five signatures simultaneously: cross-app/cross-build determinism (allocation pattern), zeroed payload (zero-init is the only zero-writer), non-zero bogus SP passing the D2 guard, boot-time re-entrancy as accomplice, fault timing independent of the R3a flag. **Falsified** if D1+D2 (below) show the fatal value never resident in any live structure and live probes never corrupt. |
| M2 | ISR-initiated switch with outer borrow live (Rev-2 carrier theory). | **Demoted to accomplice.** Re-entrancy is proven (depth=2) but R3a survived ⇒ not sufficient alone; it only widens the M1 window. Any solo theory had to survive R3a; only payload-side theories did. |
| M3 | First-run on raw stack top. | **Falsified in strict form:** first-run frames carry sentinels `0x0f`–`0x15` (`cpu.rs:85-113`), the corpse slot is zeros. Survives only as "sentinels written, then reallocation zeroed them" — a sub-case of M1. |
| M4 | Tick-storm / MLFQ duplicate-queueing. | Dead: Step-1 dedup shipped; fault byte-identical. |
| — | rflags not in saved frame (no iret) ⇒ restored tasks inherit IF from the switch site (`RFL=0x206` at death). | Not this fault; record as latent portability trap that constrains FIX-C design. |

## R3.2 Options

### DIAG-D — live-time forensics (no gate; scratch rules of D3 apply)
- **D1 live dense-probe (both apps, sequential QEMU):**
  `KERNEL=/home/ghisi/git/rosx/target/rosx/debug/rosx PHASES="wait:2;probe;wait:2;probe;wait:2;probe;wait:2;probe;wait:2;probe;wait:2;probe;wait:2;probe;wait:2;probe" PROBE_EXTRA=0x20000013b00,0x20000013c80 timeout 170s python3 -u /tmp/opencode/repro_triple_fault.py snake`
  Evidence: per-probe `info registers` RSP line + window dump. Machine test: the first probe index at which the fatal SP value appears, vs the tick stream. Note: each probe pauses/resumes the guest (timing perturbation, acceptable; `124` = inconclusive rerun per R2.2).
- **D2 stale-copy scan:** same run shape, single wide `PROBE_EXTRA=0x2000000e000,0x20000014800`. Binding machine test (POSIX grep, on the tee'd log): `grep -c '20000013b' LOG` restricted to **word-data lines** (exclude the SP register line) — a hit at address `A` identifies the structure holding the stale SP; report `A` and its field offset. Zero hits on a **pre-fault** probe falsifies M1's planted-early form.
- **D3 (fallback only)** per-phase re-entrancy readout — scratch KernelCell counter re-installed, now with the disclosed **second touch at `kernel/src/kernel.rs:286`** (chained `borrow_mut().borrow_task_mut(...)` is E0716 otherwise). Reading the counter's address into the monitor needs `nm` — **`unverified`** (Context Map does not confirm `nm` availability); route to orchestrator baseline pass before this variant is binding.
- Revert gate after every D3 edit: `grep -E 'DEPTH|high_water' kernel/src/kernel_cell.rs` empty and diff byte-equal to `/tmp/opencode/rosx_pre_r3a.diff`.

### FIX-E (NEW, recommended first fix) — slot-staged save, kernel-only, **no USER-GATE**
- Contract: add one private field `swap_save_slot: usize` to `ExecutionState` (lives inside the static `Kernel`, reachable exactly as `kernel.rs:37` already reaches `execution_state`); every `Cpu::swap_context` call site passes `&mut self.swap_save_slot`-stable pointer as `stack_pointer_to_store` **instead of** an arena-derived raw pointer; each switch method (`switch_to_task`, `switch_to_scheduler`) writes the slot back to the **outgoing** task's `stack_pointer` field at its existing epilogue, through the Option-guarded accessor (D2 guard preserved) using the handle it already holds/captures before the swap.
- Interfaces: **no signature changes** — `swap_context(stack_pointer_to_store: *mut usize, stack_pointer_to_load: usize)` (`cpu.rs:169`) already accepts any stable `*mut usize`. Kernel-only; no arch edit; no x86_32 impact.
- Expected fix power: high against M1's write side (the asm can no longer scribble into the arena); does **not** repair an already-corrupt load value ⇒ DIAG-D/D2 is the arbiter of boot-plant severity.
- Verification: `cargo xtask build && cargo test -p kernel -- --test-threads=1` (189 + new) then the R2.2 GREEN runs both apps.

### FIX-C — explicit IF ownership (USER-GATE: arch + interrupt timing)
Remove `sti` at `context_switching.S:40`; explicit `enable_interrupts()` owners at the switch epilogues (`state.rs` both methods, both resume sides) and audit `syscall.S`/SFMask path. Honest scope statement: does **not** move where switches occur (preempt still fires from ISR, `interrupts.rs:112`) and does **not** change boot-time IF ⇒ expected fix power for **this** fault is low; classify as hygiene, bundle only if the user wants the rflags-inheritance trap closed. Must preserve: `initialize_stack` has no rflags slot, so every first-run site needs the explicit-enable contract.

### FIX-B — ISR-deferred preemption (partial USER-GATE: arch + `Cpu` trait)
Timer ISR (verbatim current tail: `kernel().preempt();` at `interrupts.rs:112`) replaced by setting an arch-side `AtomicBool PREEMPT_REQUESTED` after EOI; new `Cpu` trait method `take_preemption_request(&self) -> bool`; consumed at the scheduler loop boundary (`kernel/src/scheduler/mod.rs:42-46`) and syscall return. Removes the entire class of ISR-initiated switches with outer borrow live (the proven depth-2 context). Gate scope: `arch/x86_64/src/interrupts.rs` **and** the `Cpu` trait ⇒ **`cargo build -p rosx-x86` from `arch/x86_32/` becomes a required parity gate** (x86_32 impl must implement the new method). Fairness regression accepted (notification/input-driven workload).

## R3.3 Decision trees
- **After D1+D2:** stale copy found pre-fault (or corrupt-at-spawn) ⇒ boot-planted M1 ⇒ **FIX-E now**. Corruption first visible at probe k during gameplay ⇒ window-dependent M1/M2 ⇒ FIX-E first anyway (write-side is where values get planted), FIX-B only if FIX-E is RED. No live corruption ever observed with death still occurring ⇒ re-frame to restore-site theories; escalate, no speculative fix.
- **After FIX-E:** GREEN ⇒ ship gate (R3.4); FIX-C/FIX-B become optional user hygiene. RED with byte-identical corpse ⇒ M1 falsified ⇒ FIX-B (request USER-GATE explicitly, including x86_32 parity build) as the single next attempt.
- **After FIX-B:** GREEN ⇒ ship gate + record fairness change. RED ⇒ invoke **R2.5 stop rule**: halt, full evidence report, no third speculative fix.

## R3.4 Standing bar (unchanged, restated verbatim-binding)
- GREEN (both apps, sequential): `KERNEL=/home/ghisi/git/rosx/target/rosx/debug/rosx PHASES="wait:35" timeout 110s python3 -u /tmp/opencode/repro_triple_fault.py <app>`; with `TMP=$(grep '^tmp:' LOG | tail -n1 | awk '{print $2}')`: `grep -c -E 'Triple fault' "$TMP/exc.log"` == 0 (binding), `grep -c '!!! PANIC !!!' LOG` == 0, `v=0e`/`v=08` == 0; `resets:` line void; `-9` = completed; `124` = inconclusive rerun (+50 s).
- `cargo xtask build` green; `cargo test -p kernel -- --test-threads=1` = 189 passed; `boot_and_shell` 4; `ipc_random` 1; warnings **Δ-new = 0** vs recorded fingerprint (kernel lib 65 / rosx bin 26 / shell lib 5 + 1 manifest), counting filter matching both `warning:` and `warning[EXXXX]:`; decreases at deleted sites expected, never violations.
- Scratch discipline: nothing diagnostic committed; post-scratch tree byte-equal to saved pre-R3a diff; `no_std`, no new deps, no comments (AGENTS.md).
- `unverified` and routed to orchestrator baseline: `nm`-based readouts (D3 only). All gates above use tools the Context Map confirms (`cargo`, `python3`, POSIX `grep`; no `rg`).

# Revision 4 (2026-10-11) — live-witness forensics on the load side (user option C)

Root cause only. Fixes frozen (FIX-B/FIX-C remain USER-GATE'd and untouched). Witness instrumentation is **scratch, never committed** (D3 discipline), tree restored byte-equal via the saved-diff technique. QEMU budget: **max 4 runs, strictly one at a time**. Rev-1…3 stand; nothing here repeats them.

## R4.0 Facts established by read (contract basis)

- **`Task` knows its stack at runtime — seed concern resolved, zero new fields.** The stack is inline: `stack: [usize; 2048], //16KB on 64bit systems` (`kernel/src/task.rs:56`); `Task::new` sets SP to `task.stack.as_mut_ptr().add(task.stack.len()).addr()` (task.rs:78). Base/top derive from the field itself with `usize` arithmetic only. Knowledge boundary: `prepare_entry` (task.rs:141-150) calls `cpu.initialize_stack(self.stack_pointer(), …)` and stores the returned SP back — **the kernel owns the SP value at every point in time; no `Cpu` trait change, no `arch/` edit.**
- **Swap-site audit (complete):** exactly three `swap_context` call sites — `state.rs:42` (switch_to_task), `state.rs:72` (switch_to_scheduler), `kernel.rs:179` (boot swap, still `null_mut()` save slot — pre-FIX-E path, itself a witness candidate). The trait def is `kernel/src/cpu.rs:22`; `memory/memory_manager.rs:129` is a test stub; `ports::driven::ForSwitchingTaskContext::switch_to_task` is a different trait (not a swap site).
- **Panic reaches the captured stream:** `handle_panic` prints `"\n!!! PANIC !!!"`, `"Panic at {}:{}:{}"`, `"Message: {}"`, `"System halted."` via `kprintln!` (`kernel/src/panic.rs:4-16`), then `loop {}`. Kernel console output is what the repro captures (boot banners, `[MEMORY]` lines, rendered boards are in the logs), and the R2.2 contract already greps `'!!! PANIC !!!'` in LOG — reachability is proven by the harness itself.
- **`KernelCell` has no reentrancy guard** — bare `UnsafeCell`, `borrow`/`borrow_mut` are unchecked derefs (`kernel/src/kernel_cell.rs:17-26`). Witness ordering rule follows (§R4.2: conviction line before any borrow).
- **Two seed corrections (binding):**
  1. Seed check (ii) "word at [SP] nonzero" is **wrong for mid-life frames** — `[SP]` is the saved rax slot; a legit rax can be 0. The guaranteed-nonzero word exists only in first-run frames (`initialize_stack` writes entry_point at the top slot). Replaced by: hard check = **16-word frame fully inside the target task's inline `[base, top)` window** (legit SP is always strictly inside; raw-top first-run SP = `top` ⇒ fires, which is the M3 conviction we want); soft check = zero-frame sampler (below).
  2. "Dump all live tasks" is **not expressible today**: `GenerationalArena` exposes only `add/borrow/borrow_mut/remove/replace` (`collections/src/generational_arena.rs:77-115`) — no iteration. Requires a scratch `#[cfg(debug_assertions)]`-gated iterator in `collections` (§W1.1). `Handle` fields `pub index: u16, pub generation: u16` (generational_arena.rs:7-9) print directly.

## R4.1 Witness contract (all under `#[cfg(debug_assertions)]`; the debug build `target/rosx/debug/rosx` has `debug_assertions` on — the existing `debug_assert!` at `state.rs:31` lives in that same build)

**Helper** `witness_check(direction: &'static str, handle: TaskHandle, sp: usize)` in `state.rs`:
- **No dereference before range validation** (a bogus SP may be unmapped; a deref would turn witness into fault). Order: (1) resolve target range via one `services().task_manager.borrow()`; handle-not-resolving ⇒ **hard fire** with `sp=unresolved`. (2) **HARD violation** := `sp < base || sp >= top` (note `>=`, so raw-top fires). On hard: print the conviction line **first, with no borrow held beyond field copies**, then the dump, then `panic!` — because KernelCell is unchecked (§R4.0), a dump that trips a nested borrow must never be able to swallow the conviction line.
- **SOFT violation** := `sp` in range but sampled frame is zero: `(read_volatile(sp) | read_volatile(sp + 8) | read_volatile(sp + 0x78)) == 0`, and only if `sp + 0x80 <= top`, then confirm full 16-word OR before printing (this is the corpse's "never-saved zero slot" signature, made SP-value-agnostic). Soft **prints only** (`WITNESS SOFT …`), never panics; at most one soft line per site via `AtomicBool` — false-positive-tolerant by design.
- One-shot overall: `static WITNESS_FIRED: AtomicBool` gates the hard path's dump; the conviction line always prints, panic halts after first hard fire.
- **Silent path cost:** two comparisons + (when in range) three `read_volatile` + OR. No `println` on the silent path. No new imports beyond `kprintln!`, `core::sync::atomic::{AtomicBool, Ordering}`, `TaskState` display (exists, task.rs:28-48).

**Sites** (direction strings verbatim, greppable):
| Site | File:anchor | Direction | Hard rule |
|---|---|---|---|
| L1 | `state.rs:42` pre-swap | `LOAD-TASK` | `sp` in scheduler… **incoming** handle's range |
| L2 | `state.rs:72` pre-swap | `LOAD-SCHED` | `scheduler_stack_pointer` in scheduler handle's range |
| L3 | `kernel.rs:179` pre-swap | `LOAD-BOOT` | `scheduler_thread_stack_pointer` in `main_thread_handle`'s range |
| S1 | `state.rs:45-48` write-back | `SAVE-BACK` | `saved_stack_pointer` in `outgoing_handle`'s range |
| S2 | `state.rs:60-63` write-back | `SAVE-SCHED` | `saved_scheduler_stack_pointer` in scheduler range |

S1/S2 are the **M1 planting detectors** (Rev-3 R3.1): they convert "stale save misroutes into the arena" from inference into an observed event. Borrow-safety at all five sites was audited: no outer `task_manager` borrow survives across any swap (`state.rs:24-27` temps drop pre-`:42`; `kernel.rs:37` borrows at `:38/:40` occur post-switch; `preempt` `kernel.rs:251-255` releases each `borrow_mut` before `switch_to_scheduler()`; `enqueue`/`schedule` release before returning to the scheduler loop).

**Line formats** (machine-greppable `WITNESS` prefix; panic message = the hard line):
- hard: `WITNESS HARD <direction> handle=<idx>.<gen> sp=0x<hex> task=<name> state=<TaskState>`
- soft: `WITNESS SOFT <direction> handle=<idx>.<gen> sp=0x<hex>`
- dump (per live task, via scratch arena iterator): `WITNESS TASK idx=<n> gen=<n> name=<s> state=<s> base=0x<hex> top=0x<hex> sp=0x<hex> owns=<bool>` where `owns` = hard-checked `sp` of the **bogus value** lies in that task's window — this names the owner of `0x…13b90`-equivalent at the crime moment.

**Scratch accessors:** `Task::stack_base()/stack_top()` (usize, from `stack` field); `GenerationalArena::for_each_live(&self, f: &mut impl FnMut(u16, u16, &T))` in `collections` (occupied slots, index order, yields slot index, slot generation, item ref — implementer reads the struct at generational_arena.rs:56-61 for the real field names). Marker convention: every inserted symbol contains `witness` or `WITNESS` (revert grep anchor). No comments added (scratch, but keep style); `no_std` respected; **hard boundary: zero bytes under `arch/`.**

## R4.2 Steps

**W0 — baseline snapshot (no QEMU).** `git diff > /tmp/opencode/rosx_pre_w4.diff && git status --porcelain` (record; this is the restore oracle — the current tree already contains D1+D2+FIX-E, 4 files +176/−23, 191 green per orchestrator).

**W1 — witness build (scratch; expected files touched: `kernel/src/state.rs`, `kernel/src/kernel.rs`, `kernel/src/task.rs`, `kernel/src/task_manager.rs`, `collections/src/generational_arena.rs`).** Gates, all existing commands:
1. `cargo xtask build --debug` green (release-parity compile check too: `cargo xtask build` must compile with `debug_assertions` off — all witness code excluded, no unused warnings).
2. `cargo test -p kernel -- --test-threads=1` = **191 passed** (witness code compiles under host cfg(test) — `state.rs` already does; no test executes a swap, so no witness call).
3. `cargo test -p collections` green (arena scratch fn compiles; count unchanged).
4. x86_32 parity: `cargo build -p rosx-x86` from `arch/x86_32/` (shared `kernel/` change, portable usize code only).

**W2 — snake live run (run 1 of 4):**
`KERNEL=/home/ghisi/git/rosx/target/rosx/debug/rosx PHASES="wait:35" timeout 110s python3 -u /tmp/opencode/repro_triple_fault.py snake 2>&1 | tee /tmp/opencode/w2_snake.log`
Capture: `grep -E 'WITNESS|!!! PANIC !!!' /tmp/opencode/w2_snake.log`; first-fire tick stamp from the surrounding `v=` stream. `resets:` void; exit `-9` normal teardown; `124` ⇒ inconclusive, one rerun allowed inside budget.

**W3 — tetris twin (run 2 of 4):** same invocation, `tetris`, `tee /tmp/opencode/w3_tetris.log`. Run even if W2 fires, unless W2's hard dump already fully convicts (architect call at stop-rule W4); run only if W2 was ambiguous.

**W4 — decision table (orchestrator+architect interpret before anything else — STOP RULE):**

- **(a) HARD witness FIRES** (`!!! PANIC !!!` + `WITNESS HARD` + dump table in stdout = **SUCCESS**). The `owns=` column names which task's inline stack contains the bogus SP; the direction names load vs save side; the tick stamp names the phase. Conviction is complete **iff** either (i) a `SAVE-*` hard fire shows a save landing outside its outgoing task's window (M1 planted red-handed), or (ii) the dump shows the bogus SP owned by a task other than the load target / an `sp` collision between two tasks. If only L-side fired and dump is thin, **one optional run (3 of 4)** re-runs snake with `WITNESS TASK` extended to print `entry_point` per task (scratch increment, architect-approved). **No fix ideas in this revision.**
- **(b) Witness NEVER fires, guest still triple-faults** (R2.2 `grep -c -E 'Triple fault' "$TMP/exc.log"` > 0 in the witness build). The binary split **is the verdict**: the bogus load does not pass through any kernel-side SP consumption with an out-of-range value — remaining candidates, in order, all kernel-internal first: (b1) `ExecutionState` static itself torn (witness candidate: snapshot-compare `current_task`/`scheduler`/`swap_save_slot` at each of the 5 sites; `swap_save_slot` tearing is prime suspect since FIX-E made every save flow through it — note S1/S2 already check its **value**, a torn **field address** would not be caught); (b2) double-read compare — reread `get_task_stack_pointer(handle)` immediately before each `swap_context` and panic on mismatch (catches check→use corruption); (b3) `iretq`/sysret-frame corruption — arch territory ⇒ stops at USER-GATE, report only.
- **(c) Fires EARLY/boot** (before first gameplay tick; cf. depth-2-at-boot, Rev-3 R3.0): that first fire is the prime corruption moment; capture full context, same stop rule, no fixes.
- If **only SOFT fires** and no hard, fault persists: SOFT line(s) + tick stamp + dump-on-next-hard are themselves evidence (zero-frame restore witnessed live); proceed as (a)-thin.

**W5 — revert + attestation (always last, no QEMU).** Reverse the witness to the W0 snapshot: remove/restore to `/tmp/opencode/rosx_pre_w4.diff`; then machine gates: `grep -rE 'witness|WITNESS' kernel/src collections/src` → **empty**; `git diff | cmp - /tmp/opencode/rosx_pre_w4.diff` → byte-equal; `cargo xtask build --debug` + `cargo test -p kernel -- --test-threads=1` = 191. Nothing committed at any point in R4.

**Report spec (coder → orchestrator):** verbatim PANIC block (all four banner lines + `WITNESS HARD` line) **or** the explicit negative "no WITNESS line across <N> runs; fault still present per R2.2 greps"; full `WITNESS TASK` dump table; first-fire tick stamp (`v=` neighbor) and wall phase; `TMP` dir per `tmp:` line; per-run exit codes with R2.2 greps (`Triple fault` count in `exc.log`, `!!! PANIC !!!` count, `v=0e`/`v=08` counts); `git status --porcelain` final; byte-equal attestation; every `124` rerun declared.

## R4.3 Interpretation rules (binding)

- The witness **changes code layout** — the fatal SP will likely move off `0x20000013b90` (cf. the +0x10 scratch shift already recorded). Irrelevant: checks are range/zero-frame based, SP-value-agnostic. The invariant is "frame never written in the target's own stack", not the address.
- A `!!! PANIC !!!` banner in W2/W3 stdout is the **success signal** for the experiment, not a red gate. R2.2's `PANIC == 0` gate applies only to **fix-candidate** runs, never to witness runs.
- `-9` = normal teardown; `resets:` void; `124` = inconclusive, one rerun.

## R4.4 Standing bar (unchanged, briefly)

R2.2 GREEN contract verbatim (binding = `grep -c -E 'Triple fault' "$TMP/exc.log"` == 0 etc.) — **not** applied to witness runs per R4.3; unit baseline now **191 passed** (`cargo test -p kernel -- --test-threads=1`); warning posture **Δ-new = 0** vs fingerprint `{kernel lib 65, rosx bin 26, shell lib 5, +1 manifest}`, filter matching both `warning:` and `warning[EXXXX]:`, decreases at deleted sites expected not violations; `cargo xtask build --debug` green; scratch never committed; `arch/` untouched; kernel `no_std`, no comments, no new deps.

## R5 — Witness interpretation + decisive experiment

### R5.1 Timeline reconciliation — the paradox does not exist; the fire WAS the fatal load

Read directly this turn from `/tmp/opencode/w2_snake.log` (94 lines total) and `/tmp/rosx-repro-h7s5m6k4/exc.log` (63421 lines):

1. **stdout ends at the fire.** Line 44 (`WITNESS SOFT LOAD-TASK handle=1.0 sp=0x20000013b10`) is the last kernel output before the captured fault dump; markers `game_over_snake: False`, `goodbye: False`. No board frame exists after the fire in either app (tetris: fire = stdout line 35, same structure).
2. **exc.log event numbers are QEMU's from-reset exception counter, not post-spawn survival.** Event 0 is already `v=20` (PIT runs from machine reset); events 10–2849 are ~28 s of ticks at `IP=0x22cde6 SP=0x20000013d48` (halt loop), event 2850 is the last tick on the game's stack (`SP=0x2000000f008`), event 2851 is the single `v=0e` (`RIP=0 SP=0x13b90`), event 2852 `v=08` (`check_exception old: 0xe new 0xb` precedes both), then `Triple fault`. The whole life of the guest is 2851 ticks; the fire is stdout-conterminous with event 2851.
3. **Verdict:** guest survival after the SOFT print was ~1 scheduling step — swap executed, pop-15-zeros + `ret`-to-0, `#PF→#DF→triple fault` one tick after the announced load. Option C got exactly what it was built for: the crime witnessed in life, once, and it is the death itself. (The "~1400 events later" figure was an event-counter misread; the 40-s prompt-burn model is likewise dead — `prompt_seen: True`.)
4. **Consequence for the one-shot latch:** a survived zero-frame load is physically impossible (RIP=0 is instant), so the first (and only) SOFT fire at site L1 **is** the first and only zero-frame load — there is no hidden earlier benign fire.

### R5.2 What is now established (beyond Rev-3/4)

- **E1 — the poison is a field VALUE, not an unmapped SP, and no save ever executed at it.** SOFT proved: in-range, 16-word window all-zero, at the load instant. S1/S2 saw every write-back with a nonzero (real) frame — so no legitimate save ever produced or touched `[0x20000013b10, +0x80)`. A real save at an SP always leaves that window nonzero (15 register words + the return-address word on the CPU stack at entry). 
- **E2 — FIX-E persistence remains the strongest structural fact.** Current tree writes `Task::stack_pointer` only through setters (`Task::new` task.rs:79; `prepare_entry` task.rs:148; S1 `set_task_stack_pointer(outgoing, swap_save_slot)` state.rs:45-48; S2 scheduler write-back state.rs:59-63). The asm no longer writes through arena refs at all — and the corpse is byte-identical pre/post-FIX-E. **Any theory whose mechanism is the removed save-side raw-ref is dead.** Common factors across both trees shrink to: boot/spawn-time frame materialization (`initialize_stack`/`prepare_entry`), the scheduler's value plumbing (`swap_save_slot` contents), and the heap itself.
- **E3 — identity arithmetic points at the idle task, not the game (coder's label unverified).** The soft line carries no task name. The halt stream ran ~28 s at `0x13d48`; if that window's top is `0x13d90`, the first-run frame is `[0x13d10, 0x13d90)` and **poison = first-run-SP − exactly 0x200 = exactly four 16-word frames**, still inside the same window (consistent with the witness's in-range finding) but 0x200 below the deepest address the halt loop ever used — genuinely never-executed territory. The task the scheduler would pick right after the game blocks (no further input in the repro) is precisely the idle task. This must be **named, not assumed** — R5.4 W-a does it.
- **E4 — determinism is now a writer constraint:** snake vs tetris differ in game size/code yet produce the identical poison value at the identical relative moment (first pick of handle 1.0 after the game blocks). The poison does not depend on game content, only on boot/spawn allocation order and scheduling structure.

### R5.3 Ranked mechanisms (each with the falsifier run 3 settles)

- **C1 — Re-zeroing of a legitimately written frame: overlapping `Box<Task>` allocation.** `Task::new`'s `stack: [0; 2048]` (task.rs:73) is the tree's only zero-writer of a 16 KiB stack region; if the freelist (memory/memory_manager.rs) ever hands out a chunk overlapping a live task's window (coalescing/split bug), the fresh `Task::new` zero-inits over the live task's first-run frame while gen-0 handles stay valid (no free needed — double-hand-out is enough). The spawn burst at ~t+27 s (ELF bytes box, image, task box) is the natural trigger; poison visible at first re-pick afterwards. **Falsified if** run-3 boot/spawn PREP witness reports `frame_or == 0` already at write time (C2), or the run-4 overlap trace is empty across the whole life.
- **C2 — Frame never materialized: `prepare_entry`/`initialize_stack` wrote elsewhere or twice.** If the 16 sentinel writes (`0x0f..0x15` + entry, cpu.rs:82-113) did not land at `[returned_sp, returned_sp+0x80)` of the same box whose field received `returned_sp` — wrong top, double `prepare_entry` (second call re-derives from the field: `top−0x80` then `top−0x100`, leaving the field's old frame value orphaned — check call count), or box identity confusion — the frame region stays zero-init forever. **Falsified if** the PREP witness prints nonzero `frame_or` at every prepare, for handle 1.0.
- **C3 — Phantom direct write to the field bypassing setters (residual alias).** Weakened hard by E2 (FIX-E removed the asm path) but not eliminated: `get_task_stack_pointer_ref`-style refs still exist; any stale pointer write is invisible to range-validated setters. **Falsified by** run-3 shadow/field mismatch staying silent (shadow == field at every load ⇒ no direct writes).
- **C4 — Handle/value mix-up inside scheduler bookkeeping (a setter run with correct target but value belonging to another world).** A write-back whose `self.current_task` names idle while the running context was elsewhere would plant some other context's rsp — but any such rsp lands in *that* context's window; the poison is in **idle's** window at a never-executed address, which a live rsp cannot be (E1). Kept last. **Falsified by** run-3 writer-tag: last legit writer for handle 1 shows what was written and when.
- **C5 (dead) — timing/ISR theories, recycled-slot theories.** R3a (Rev-3), byte-identical corpse under FIX-E (E2), and gen 0 never recycled (Rev-4 fact 4) kill all three. **Implication: FIX-B/FIX-C are evidence-dead ends** — both change ISR timing; the poison is a data-planted-at-boot/spawn artifact that fires on an ordinary scheduler pick. Recommend formally demoting both gates unless run 3 unexpectedly resurrects a timing dependency.

### R5.4 Run 3 (of 4) — witness v2: name the writer at the crime scene

Scratch rules unchanged (`#[cfg(debug_assertions)]`, `witness`/`WITNESS` markers, D3 discipline, save `/tmp/opencode/rosx_pre_r5.diff` from the current 191-green tree, revert + byte-equal attestation after). Additions to the v1 site set (keep L1/L2/L3/S1/S2 exactly as in Rev-4):

- **W-a PREP witness** (kernel-only; `prepare_entry` is the single funnel, task.rs:141-150): after `set_stack_pointer(new_stack_pointer)`, OR the 16 words at `[new_stack_pointer, +0x80)` and print `WITNESS PREP idx=<n> gen=<n> name=<s> sp=0x<hex> frame_or=0x<hex> call_n=<n>` (per-handle call counter; ≤ ~8 lines total per run). This is the **C1-vs-C2 binary split**: nonzero at write + zero at death ⇒ re-zeroed (C1); zero at write (or repeated `call_n`) ⇒ never materialized (C2).
- **W-b writer tag + shadow** (kernel-only): static `WITNESS_TAG/WITNESS_SHADOW` indexed by handle index; updated inside the four legitimate writers (Task::new, prepare_entry, `set_task_stack_pointer`); at L1 (before load) print on **every** check-visit… no — print only when `shadow != field` (`WITNESS DRIFT idx=<n> shadow=0x… field=0x… tag=<writer id>`), which is the C3 detector: a mismatch can only come from a direct memory write bypassing setters. Silent otherwise.
- **W-c soft dump extension:** change the SOFT branch to fire **once globally** and, on that fire, print the existing task-table dump (re-use the v1 hard dump fn) **plus** the raw 32-word hex window around `sp` (±0x100, via existing volatile reads) and handle 1.0's PREP line replay. One fire, one run, full provenance: owner names + ranges (settles E3's identity question: does `[0x13b10, 0x13b90)` live in the window of the task that halted at `0x13d48`, and is `0x13b90` that window's `top − 0x200`?), plus whatever residue neighbors the zero frame.
- Build gates identical to W1 (Rev-4): `cargo xtask build --debug`, `cargo xtask build` (release-parity), `cargo test -p kernel -- --test-threads=1` = **191**, `cargo test -p collections` = 21, x86_32 parity from `arch/x86_32/`. Run command unchanged: `KERNEL=/home/ghisi/git/rosx/target/rosx/debug/rosx PHASES="wait:35" timeout 110s python3 -u /tmp/opencode/repro_triple_fault.py snake 2>&1 | tee /tmp/opencode/w5_snake.log`. **Run snake only**; keep tetris + one reserve for run 4.
- **IP mapping: not needed before run 3.** `0x22cde6`/`0x2aa053`/`0x227c1a` attribution (scratch rebuild + `addr2line` on host binutils — worked earlier per session facts) is post-hoc and must not spend QEMU time.

**Run-3 decision tree → run 4:**
- **X — PREP nonzero for handle 1, DRIFT silent, soft dump zero-window at death ⇒ C1 convicted.** Run 4: allocator overlap trace (scratch: at every `alloc ≥ 0x1000`, compare the returned chunk against the recorded ranges of all live task windows; print `WITNESS OVERLAP` on any hit; free-path tag for C1's coalescing variant). Expect the overlap event between spawn and death. **Fix path then is kernel-only (`memory/memory_manager.rs`) — no USER-GATE, and FIX-B/FIX-C stay dead.**
- **Y — PREP zero (or double `call_n`) ⇒ C2 convicted.** Run 4: pin the write target — print box addr, `stack` field addr, `sp` in/out of `prepare_entry`, and a sentinel read-back at boot for the idle task specifically. If the culprit turns out to be inside `initialize_stack` (arch), the **fix** needs a USER-GATE — but run 3/4 remain kernel-only.
- **Z — DRIFT fires ⇒ C3 convicted;** tag + shadow-vs-field names the last legit writer; run 4 = pre/post snapshot around that writer to expose the stale-ref holder. Kernel-only fix likely.
- **W — none of the above (PREP nonzero, no drift, soft shows a NONZERO window at death — i.e., SOFT itself stops firing):** means the zero state is not persistent ⇒ layout-sensitive race; only then do we reopen timing and re-consult the FIX-B/FIX-C gates. Unlikely given E4 determinism.
- **Stop rule (R2.5 unchanged):** if run 4 also fails to convict, halt and report; no fifth run, no speculative fix.

### R5.5 Fix stance + standing bar

- **No kernel-only fix is defensible today** — the writer is unnamed (X/Y/Z decide which); FIX-B/FIX-C are formally demoted per R5.3 with the W-branch exception.
- Standing bar unchanged: R2.2 contract verbatim (binding `grep -c -E 'Triple fault' "$TMP/exc.log"`, `resets:` void, `-9` teardown, `124` = one rerun); unit baseline **191**; collections 21; warning posture **Δ-new = 0** vs fingerprint `{kernel lib 65, rosx bin 26, shell lib 5, +1 manifest}`, both `warning:` and `warning[EXXXX]:` forms counted; scratch revert = grep-clean `witness|WITNESS` + `git diff | cmp -` byte-equal to the saved pre-run diff; one QEMU at a time; `arch/` untouched; no commits; kernel `no_std`, no comments, no new deps.

## R7 — Allocator audit: the convicted defect + fix contract

**Scope read:** `kernel/src/memory/free_list_allocator.rs` (full), `kernel/src/memory/memory_manager.rs` (full), `kernel/src/task.rs`, `kernel/src/task_manager.rs`, `kernel/src/syscall.rs`, `kernel/src/elf/mod.rs`, `kernel/src/ipc/*`, `kernel/src/future.rs`, `kernel/src/keyboard.rs`, `collections/src/generational_arena.rs`, `usrlib/src/syscall.rs`, app `#[global_allocator]` sites, and the run-4 trace `/tmp/rosx-repro-c1wn_yxh/stdout_full.log`. (`BitmapChunkAllocator` exists but is not compiled into the active path; the single static allocator is `MemoryManager → FreeListAllocator`.)

### 1. Header identity and the 8-byte skew — settled

`free_list_allocator.rs:7-8`: `const BLOCK_HDR: usize = 16;` and `const ALLOC_HDR: usize = 32;`. The 0x28 base observed in run-4 prints (`FREE p=… base=p+0x28`) is **not header arithmetic**: geometry proves printed `p` is the *usable pointer* and printed `sz` the Rust *layout size* — base = p+0x28 is the `stack` array offset inside the 0x4030-byte `Task` (`task.rs:56-58` `stack: [usize; 2048]`). Consequences: (a) the sub-minimum `FREE sz=0x8/0x10/0x18/0x20` churn at `0x1e308`/`0x1e3d8` is **legal** small-object traffic, not direct corruption proof; (b) the 8-byte skew between the frame writer and slot reader is a **write-site offset mismatch**, not header/alignment; the allocator's own split math (`allocate` :77-100, `remaining >= BLOCK_HDR` guard) is exonerated.

### 2. Convicted defect: unvalidated free path in `FreeListAllocator` (F1 + F2 + F3)

**F1 — `deallocate` (`free_list_allocator.rs:104-122`) performs no membership validation, and its unlink writes the list even on a miss.** Verbatim shape of the unlink:

```rust
let mut prev: *mut AllocHeader = core::ptr::null_mut();
let mut current = self.alloc_head;
while !current.is_null() {
    if current == header { break; }
    prev = current;
    current = unsafe { (*current).next };
}
if !prev.is_null() { unsafe { (*prev).next = (*header).next; } }
else { self.alloc_head = unsafe { (*header).next }; }
```

A double free of a recycled pointer misses the walk with `current == null` and `prev` = tail node, then `(*prev).next = (*header).next` **plants whatever `next` the reused memory now holds into the tail of the alloc list** (or into `self.alloc_head` when the list held only that node). `insert_free_block` is then called **unconditionally** — the corruption of the alloc list survives even if the free insert were guarded.

**F2 — `insert_free_block` (`:142-176`) has no duplicate/overlap rejection.** Coalescing tests only exact adjacency (`:163` `block_end == next as usize`, `:171` `prev_end == start`). A second descriptor over an already-live range is inserted, and `allocate` first-fit (`:62-100`) then hands the same bytes out while the first owner still uses them.

**F3 — reachability.** `SyscallNum::Dealloc` (`syscall.rs:70-74`) passes a user-controlled pointer into `MEMORY_MANAGER` with no owner/range check, and every ELF app installs a `#[global_allocator]` proxying user `Box`/`Vec` traffic into this **one shared kernel heap** (`apps/snake/src/main.rs:15-20`, `apps/tetris/src/main.rs:14-19`, also shell/conway/hello_elf). Owner enforcement is dead code: `memory_manager.rs:83` hardcodes `BlockOwner::Kernel`, and `deallocate_by_owner` (`free_list_allocator.rs:124-140`) has zero production callers.

**Mechanism for the run-4 crimes.** The only bulk zero writers in reach: `Task::new` `stack: [0; 2048]` (`task.rs:73`) and the ELF image buffers `vec![0u8; max_addr as usize]` (`elf/mod.rs:85`, `:140`) with `copy_from_slice` fills (`:92-93`, `:147-148`). One stale double free of a recycled chunk (snake-startup **task box, layout 0x4030**, at the `0x13a90` region — the same arena-reused slots that became idx2/idx3) re-enters the free list; a later allocation overlaps `[0x13a90, ~0x13e00]` and:

- **2a:** `Task::new` zeroes the reused region including the sp slot (initial `stack_offset` 0). The frame writer's zero-guard (`kernel_cell.rs:181-190`) then publishes the Idle frame at `*slot == 0` → `base + 0x208` = **legit-looking sp 0x13b10, fully zeroed incl. the RIP slot**; overlapping re-zeroing later leaves the pattern all-zero (consistent with run-4), and the corpse top = `base + 0x208 + 0x28` = **0x13b38** ✓.
- **2b:** one overlap chunk ≥0x370 spanning `[0x13a90, ~0x13e00]` covers idx2 `name@0` + sp@+0x20 and idx3's header → `name=''`, idx2 `sp=0x0`, tops intact (overlap tail lands mid-region) ✓.
- **No 2c DRIFT** because sp was zeroed *through memory*, never via setters: a zeroed-sp idx2 returns at the `switch_to_task` zero-guard before `witness_drift_check` ✓.

### 3. Ranked competing defects, each with falsifier

1. **F1/F2 (convicted)** — falsifier: post-fix build still triple-faults at the same RIP.
2. **Free-list-node overflow** (inline metadata, 8-byte overrun from an adjacent live block writing node `next` fields) — falsifier: same as above; this is the escalation theory if (1) survives its own fix. Falsifier triggers one confirmation run.
3. **Frame-writer/slot-reader 8-byte skew as an independent memory bug** — falsifier: unit-level — frame writes land exactly in `stack[2047]`-down slots; post-fix GREEN with the guard at `kernel_cell.rs:181-190` untouched.
4. **Exonerated by reading** (no falsifier owed): `allocate` split arithmetic; `GenerationalArena` (`remove`/`replace` generation-guarded; `add` not bumping gen is intended); `task_manager::remove_task`; `Kernel::wait_future` single `consume` (`kernel.rs:225-233`); keyboard/IPC/mailbox `Box` lifecycles (leak risks only); `Dealloc` arg order `(ptr,size,align)` matches `usrlib/src/syscall.rs:77-82`.

**Code conviction suffices — no extra forensic run.** F1/F2 are validation absences provable by reading, and the fix is self-attesting: rejection converts the first bad free into a `BAD-FREE` log line at the call site, so the ship gates double as confirmation. Forensic budget stays 4/4. Only falsifier (1)→(2) would justify one window-filtered raised-cap run.

### 4. Minimal fix contract (kernel-only, `kernel/src/memory/free_list_allocator.rs`)

1. **`deallocate` — walk with `prev_next: *mut *mut AllocHeader`** (init `&mut self.alloc_head`, advance `&mut (*current).next`), so a miss *cannot* write the list. On miss: `kprintln!("BAD-FREE ptr=0x{:x} size=0x{:x}", …)`, `debug_assert!(false, …)`, `return` with **no list mutation**. Pre-validate before any arithmetic: non-null, `BLOCK_ALIGN`-aligned, `(*header).size >= ALLOC_HDR + BLOCK_ALIGN` (= 0x28); any violation → same BAD-FREE rejection path.
2. **`insert_free_block`** — return `bool`; reject (no mutation) if the new range overlaps **any** free entry (`start < e_end && e_start < end`, which subsumes duplicates) or `size < BLOCK_HDR`. Exact-adjacency coalescing (`:147-151`, `:163`, `:171`) unchanged.
3. **Validation is unconditional** — active in release too (corruption must not survive a `--release` ship build).
4. **`memory_manager.rs` unchanged.** `used` bookkeeping (`:107`) will drift by rejected sizes — cosmetic; do not "fix" unless trivially clean. `SyscallNum::Dealloc` owner tagging is advisory follow-up, **out of scope**.
5. Optional: `#[cfg(debug_assertions)]` disjointness helper over alloc/free lists for test assertion.
6. Formatting `0x{:x}` only (output backend verified non-allocating — a `{}`-path that allocates inside `deallocate` would self-recuse).

### 5. Unit-test additions (`#[cfg(test)]` in `free_list_allocator.rs`)

Extend existing tests with: (a) **split adjacency** — post-split alloc+free ranges partition the block exactly, no overlap; (b) **coalesce exact-adjacency** — left/right merges yield exactly one entry of expected size. New, **must fail today, pass post-fix**: (c) `stale_double_free_after_realloc_does_not_release_live_block` — alloc A, free A, re-alloc (recycles over A), free A again → rejected, second owner's bytes never re-issued; (d) `double_free_rejected_and_allocations_stay_disjoint`; (e) `dealloc_foreign_pointer_leaves_allocator_intact` (lists byte-identical); (f) `corrupted_header_size_rejected` (`size < 0x28`, misaligned, null); (g) deterministic LCG stress: every live alloc pair disjoint, free ⊥ alloc.

### 6. Acceptance (standing bar, unchanged)

- `cargo test -p kernel -- --test-threads=1` — baseline claim 191 passed (pre-change baseline multiset; new tests add on top); `cargo test -p collections` (21, baseline claim).
- `cargo xtask build --debug` **and** `cargo xtask build` (release carries the same validation — §4.3); `cargo build -p rosx-x86` from `arch/x86_32/` — **parity mandatory**, `memory/**` is shared by both arches.
- Warnings: **Δ-new = 0** vs recorded fingerprint {kernel lib 65, rosx bin 26, shell lib 5, +1 manifest}, matching both `warning:` and `warning[EXXXX]:`; decreases at deleted sites expected, attributed by multiset diff, never violations.
- §R2.2 gate, both apps GREEN: `KERNEL=/home/ghisi/git/rosx/target/rosx/debug/rosx PHASES="wait:35" timeout 110s python3 -u /tmp/opencode/repro_triple_fault.py <app>`; then `TMP=$(grep '^tmp:' LOG | tail -n1 | awk '{print $2}')` and `grep -c -E 'Triple fault' "$TMP/exc.log"` == 0, `!!! PANIC !!!` == 0, `v=0e`/`v=08` == 0; exit `124` → rerun +50s; sequential QEMU only.
- Ship gate: un-`#[ignore]` `app_lifecycle` 2 (`tests-integration/tests/app_lifecycle.rs:16`, `:58`) → **app_lifecycle 2 passed**, plus `boot_and_shell` 4 + `ipc_random` 1. Exact `cargo xtask test --integration` flag set remains **unverified** — orchestrator baseline must establish the verbatim command before binding.

### 7. Dispositions

- **FIX-E: KEEP** (tree state in `state.rs` is the witness plumbing; unrelated to convicted path).
- **FIX-B / FIX-C: DEAD** — never implemented; formally retired by this audit; no gates owed.
- **`Task::stack_pointer_mut` (`task.rs:90`): REMOVE**, grep gate: only the definition survives (all production readers go through the switch path); zero writers were found — removal is the grep-provable claim.
- D1 (MLFQ dedup) / D2 (Option SP guard): **KEEP**.


## ORCHESTRATOR ERRATUM on R7 (2026-10-11, appended with verification)

- R7 section 2's illustrative mechanism paragraph cites a "frame writer's zero-guard (kernel_cell.rs:181-190)" — that file is 33 lines total and contains no such guard; frame saves are performed by `arch/x86_64/src/context_switching.S` asm. Its corpse arithmetic target 0x13b38 does not match the observed 0x13b90. That paragraph is NOT endorsed; do not build on it.
- The F1/F2/F3 convictions themselves are INDEPENDENT of that paragraph and were re-verified read-only by the orchestrator: deallocate's unlink writes `(*prev).next = (*header).next` / `self.alloc_head` even when the walk misses (`kernel/src/memory/free_list_allocator.rs:104-122`), then calls `insert_free_block` unconditionally; `insert_free_block` (:142-176) has no duplicate/overlap rejection and coalesces only on exact adjacency; `SyscallNum::Dealloc` (`kernel/src/syscall.rs:70-74`) takes an unvalidated user pointer into the shared kernel heap; `BlockOwner::Kernel` is hardcoded (`kernel/src/memory/memory_manager.rs:83`); `deallocate_by_owner` has no production callers in the active allocator (matches exist only in `bitmap_chunk_allocator.rs`, which is not on the active path).
- The fix idiom for F1 already exists in the same file: `deallocate_by_owner` (:124-140) uses the correct `prev_next: *mut *mut AllocHeader` walk. `deallocate` should follow it.
- The escalation clause (BAD-FREE never appearing post-fix ⇒ defect (2), one confirmation run) stands.

## SCOPE REVISION (user directive, 2026-10-12) — memory-only ship

- The ship scope is the R7 memory-management fix ONLY (`kernel/src/memory/free_list_allocator.rs`).
- D1 + D2 + FIX-E are REVERTED from the working tree (tree byte-clean at `97be6b7`, unit baseline back to 182). They are parked as future work: `plans/scheduler-lifecycle-hardening.md` + code snapshot `plans/scheduler-lifecycle-hardening.patch` (verified `git apply --check` clean on the reverted tree). R7 §7's "KEEP D1/D2" disposition is superseded by this directive.
- Fix-round gates unchanged except the unit baseline is now **182** (+ new freelist tests on top).

## R8 — Post-R7 verdict: dealloc path exonerated by silence; single-run discriminator

### 1. Interpretation — what the three facts jointly imply

BAD-FREE=0 (entry validation silent), debug-only (release GREEN on **unfixed** HEAD), zeroing-signature (bulk `Task::new`/ELF zero-init landing on live memory) jointly say: **the poison never passes through `deallocate`/`insert_free_block` as an argument** — it is either (a) handed out by `allocate` while still owned, or (b) written out-of-band. The re-entrancy hypothesis: **refuted from source, with one residual tail** — verify, don't assume:

- `memory_manager.rs:71-92/:94-111` (quoted shape): every entry reads IF and `cli`s before the walk when IF=1. Task-mode walks (IF=1→cli) are IRQ-safe; IRQ-context walks (timer path genuinely allocates: `timer.rs:25` `entry(deadline).or_insert_with(Vec::new)`, `pop_expired` returning `Vec`, `wake_tasks(alloc::vec![...])`, `scheduler.rs:111` `interrupt_drain_buf` growth) run with IF=0 on entry — so nesting is blocked there too. Syscall entries are IF=0 by `SFMask::write(INTERRUPT_FLAG)` (`arch/x86_64/src/cpu.rs:52`); `are_interrupts_enabled` is a genuine pushfq read (`cpu.rs:64-66`); the only `sti` in arch is `context_switching.S:40`, which fires on the **target** task's stack after the switch (switch path is non-allocating). Single CPU ⇒ mid-walk IRQ re-entry is impossible.
- Residuals: (i) the check→cli window (`:72-75`) is real but only ever produces **sequential** handler-then-walk overlap (harmless); (ii) a faulting walk enables true mid-walk re-entry via the exception handler — but that needs pre-existing corruption, so it cannot be the first cause; (iii) prior "KernelCell depth ≥4" measured **all** cells (nested cpu/state borrows inside `alloc`'s own windows, `:73/:75/:86`), not allocator-cell exclusivity — it is not evidence of allocator re-entry.

Ranked hypotheses:
- **H1 — allocate-side double-handout** (structural, no bad free; activated by debug-specific sizes/fragmentation: split boundary, `remaining` edge, region adjacency). Fits everything: entry guards never convict, `Task::new`'s `[0;2048]` (`task.rs:73`) or `vec![0u8; max_addr]` (`elf/mod.rs:85,:140`) is the zero writer into the overlapping live range, release survives because debug layouts/sequence differ. **Current front-runner.**
- **H2 — out-of-band writer bypassing the allocator** (debug-fat task-stack frames overflowing the 16 KiB `stack: [usize;2048]`; ELF `copy_from_slice` overrun). Classic debug-only discriminator. Weakness: overruns write non-zero frame data, while the crime scene is zeroed.
- **H3 — in-walk re-entry** (orchestrator's candidate). Source-refuted above; keep a zero-cost latch in the run to seal (i)/(ii) empirically.
- **H4 — `is_setup=false` boot-window walks** (`:72` short-circuit): unguarded, but boot is pre-IDT single-threaded and boot-phase tests are green. Refuted by timing; no run cost.

### 2. One-run confirmation spec (snake, debug, scratch; revert after, byte-equal attestation per Rev-4/5)

Kernel-side only (no `arch/` edits — USER-GATE stays untested). Coder: save pre-hashes of touched files + `/tmp/opencode/rosx_pre_r8.diff` oracle first. Four hooks, all cheap, latch-and-continue, hard print caps, nothing inside the list walk itself:

1. **Re-entry latch** (`MemoryManager::alloc/dealloc`): `static ALLOC_DEPTH: AtomicUsize` incr/decr; on entry if depth>1: latch counter, print at most 3 lines `R8-REENTRY d=.. ptr=0x{:x} if=..`; final depth high-water printed once at fault/shutdown path.
2. **Big-handout disjointness gate** (wrap the `Ok(ptr)` arm at `:88-91`, after interrupts restored): scratch table of live ranges (append on alloc, mark dead on dealloc, fixed capacity, drop-oldest). For **allocations ≥0x1000 only** (Task box 0x4030, ELF image buffers — the convicted zero writers) scan all live entries; on overlap print once `R8-HANDOUT new=0x{:x}-0x{:x} old=0x{:x}-0x{:x} sz=0x{:x}`, latch. Small allocs: skipped (timing budget). Also print every big handout, capped 40 lines: `R8-BIG ptr=0x{:x} sz=0x{:x}` — timestamps `Task::new` zero-init against the fault window in stdout order.
3. **Correlation ring**: last 32 ops `(op, ptr, size)`; existing base-witness prints (v2 infra) dump the ring on the fatal path.
4. **SP low-watermark**: in the kernel switch path before `swap_context`, compare incoming sp vs the task's stack-array base; print once per task `R8-SPLOW sp=0x{:x} base=0x{:x}`.

Run: existing §R2.2 command unchanged (`PHASES="wait:35"`, snake debug). Expect corruption to still occur — this run diagnoses, it does not fix.

### 3. Verdict table

| Run output | Convicts | Refutes | Next |
|---|---|---|---|
| `R8-HANDOUT` line with two owners | **H1** (allocate double-handout) | H2, H3 | R9: targeted `allocate`/split/free-list audit at logged sizes; kernel-only |
| `R8-REENTRY` latch | **H3** (residual (i)/(ii) live after all) | H1 if no HANDOUT | Fix = unconditional cli-at-entry in `MemoryManager` (kernel-only, trait exists) — no arch change unless mechanism traces into `syscall.S`/IDT → then **USER-GATE request** |
| `R8-SPLOW` | **H2** (stack overflow) | H1 (verify no HANDOUT co-fires) | Enlarge stack / debug-frame diet, kernel-only; guard-page needs page tables → **USER-GATE flag** |
| Nothing fires, base witness still shows zeroing + identical corpse | All in-allocator mechanisms | — | Out-of-band stores confirmed; poison-sweep escalation = **second run — budget spent → stop-and-report per D4** |

### 4. Disposition

- **KEEP the +270/−9 validated-deallocate fix** in the tree during the confirmation: silent BAD-FREE proves zero false rejects, tests green, and the dealloc-path exoneration depends on it staying in place.
- FIX-B/FIX-C: remain dead; nothing in this round touches `arch/`. Any guard-page/IDT/`.S` follow-up is a **USER-GATE request**, flagged as such, never a written contract.
- Unit bar: **192** kernel (`cargo test -p kernel -- --test-threads=1`) + 21 collections, Δ-new=0 vs recorded fingerprint, debug+release+x86_32 parity — enforced on the **committed** tree only; while scratch hooks are in the tree, no gate runs except the diagnostic itself. Scratch fully reverted after with byte-equal attestation (file hashes vs pre-hashes, `git status --porcelain kernel/` empty).

### 5. Gate posture

If the run convicts H1 → R9 is kernel-only `free_list_allocator.rs` work, no user gate. If it convicts H3 with a syscall/IDT-side mechanism, or if mitigation requires a stack guard page (page-table edit): I will **frame the user-gate request** (what changes in `arch/`, minimal diff, why kernel-side is impossible) rather than contract it. No commits from this round; budget after this run: **0 — D4 applies on any null result.**

# R9 — Static defect audit (read-only, current source at `34f17b7`)

Method note: every line ref below re-read this turn from current source; no old numeric maps reused.

## 1. `kernel/src/memory/free_list_allocator.rs` — allocate side

**Split arithmetic: EXONERATED (arithmetic, both arches).** Constants are derived, not literal: `BLOCK_HDR = size_of::<FreeBlock>()`, `BLOCK_ALIGN = align_of::<FreeBlock>()`, `ALLOC_HDR = size_of::<AllocHeader>()` (:24-26). x86_64: 16/8/32. x86_32: 8/4/16. `let needed = (ALLOC_HDR + usable).max(BLOCK_HDR);` (:90): `ALLOC_HDR > BLOCK_HDR` on both arches ⇒ the `.max()` arm is unreachable; `needed` is a multiple of `BLOCK_ALIGN`; return `start + ALLOC_HDR` is align-satisfying (aligns > `BLOCK_ALIGN` rejected :85-87). Split `if remaining >= BLOCK_HDR` (:100): remainder `[start+needed, start+block_size)` is disjoint from used `[start, start+needed)` by construction; `(*split)` sits ≥ `ALLOC_HDR` past `start` so header writes (:111-114) cannot clobber it; exact-fit/`else` path (:106-109) hands the whole entry. Free entries never straddle region boundaries from `new()` (:60-73, one entry per region); coalesce (:226-241) merges only on exact address equality, i.e. only truly contiguous memory. The allocate-side splice (`*prev_next = split` / `*prev_next = (*current).next`) mutates the list but never *introduces* a range — it is range-preserving, so absence of the #59 admission check there is correct, not a gap.

**A1 (top finding) — the admission model is alloc-blind: a range covering a LIVE allocation can enter the free list with BAD-FREE=0.** Provable by reading:
- `insert_free_block` probe (:198-207) — `if start < probe_start.saturating_add(probe_size) && probe_start < end` — compares the inserted range **only against other FREE entries**. A live `AllocHeader` block is invisible to it.
- `deallocate` guards are: `region_contains(start, block_size)` (:138) and a membership walk matching **header address identity only** (:143-152, `while !current.is_null() && current != header`) — no caller/ownership context, and `block_size` comes from the trusted in-band `(*header).size`.
- Reach: `SyscallNum::Alloc` returns raw kernel-heap pointers to user space (`(unsafe { services().memory_manager.alloc(layout) }) as usize`, `syscall.rs:69`) and `SyscallNum::Dealloc` (`syscall.rs:71-74`) feeds any user pointer into that identity walk — any pointer `p` with `p-32` equal to a live alloc header satisfies every check and releases a block its true owner is still using. Snake/tetris route **all** `Box`/`Vec` traffic here (`apps/snake/src/main.rs:14-25` is a raw pass-through; shared heap — R7 F3 unchanged by #58/#59).
- Consequence: exactly our signature. A validated-then-blinded release makes a boot-era range (idle's `Box<Task>`, handle 1.0 = arena idx1 created at `kernel.rs:153-158`) re-handable; the next bulk owner (`Task::new` `stack: [0; 2048]`, task.rs:72, or `vec![0u8; max_addr]` zero-fill) legally scribbles zeros over the live idle frame; the SP field can survive if the overlap is partial — matching "plausible SP + zeroed 16-word frame". The existing `bad_free` can **never fire for this crime class** — BAD-FREE=0 is expected under A1, not exoneration.
- Falsifiable: post-fix (cross-probe vs alloc list, §5) the next repro run must print `BAD-FREE overlaps-live-alloc ...` at the contamination instant if A1 is the carrier; zero such lines with identical corpse ⇒ falsified.

**A2 — R8's run did not test A1 (evidence-side defect).** The big-handout gate (R8 §2.2) built its live table by appending allocations observed **after hook install** and skipped small allocs; boot-era live blocks (idle, scheduler, early boxes — the exact victims) were never in the table, and the gate scanned alloc×alloc only at handout time, never insert×live. "No big-vs-big overlap" therefore does **not** refute H1. Cheap retrofit: seed the table once from a walk of `alloc_head` (it lists every live block since boot) and `head` (free list) at hook install.

**A3 — `new()` seeds free entries with no admission probe** (:58-79): overlapping boot regions ⇒ day-0 double handout, invisible to #59. Partially mitigated: `arch/x86_64/src/main.rs:91-98` accepts only `MemoryRegionKind::Usable`, which the multiboot map lists disjointly; also `memory_blocks.count` overflow (>32 Usable regions) would panic, not wrap. Low suspicion; host-testable from recorded `[MEMORY] Region:` lines (`memory_manager.rs:58-63`) in any existing log — no new run needed.

**A4 — `deallocate_by_owner` (:171-187)** inserts `(*current).size` with no region/size validation and discards `insert_free_block`'s bool. Grep this turn: zero production callers (only `#[cfg(test)]` + the unused `bitmap_chunk_allocator.rs`) — hygiene, not active.

## 2. `kernel/src/elf/` — image sizing

`max_addr` is the MAX over PT_LOAD of `p_vaddr + p_memsz` (`elf/mod.rs:71-79`, `:126-134`) — not MIN. Buffer `vec![0u8; max_addr as usize]` (:85, `:140`) covers every segment start and the `p_memsz > p_filesz` tail (already zero from the vec — correct bss semantics). The copies (:92-93, `:147-148`) use **slice indexing + `copy_from_slice`**: out-of-range `p_filesz`/`p_vaddr` combinations **panic, not scribble** — the load path cannot write past the buffer under any segment combination; malformed ELFs (which the four shipped release-built app ELFs are not — they run) become a kernel panic/DoS, not an out-of-band writer. Residual unchecked: relocation writes `elf_arch.apply_relocation(base, rela.r_offset …)` (:198, impl `arch/x86_64/src/elf_arch.rs:8`) are raw `base+offset` with no `< image.len()` bound, and the dyn/rela table reads (:174-177, :192-196) run unbounded when `PT_DYNAMIC` is not fully covered by a PT_LOAD. For the shipped static-pie app ELFs (relocations demonstrably work) offsets are in range. Supports H2 only for malformed input; not a snake/tetris vector.

## 3. `task.rs` + arena (post-#58)

`Box::new(Task { … stack: [0; 2048], … })` (task.rs:65-74) writes exactly `size_of::<Task>` — no box-size overrun. Gen checks in `arena.rs` are complete on all four paths (`borrow` :45, `borrow_mut` :53, `remove` :61, `replace` :72). One structural sharp edge: `replace` (:70-77, `self.items[index] = Some(item);`) **drops the old item and keeps the generation**, but its only caller is the future arena (`future.rs:157`) — never `TaskManager` (grep this turn) — so idle's `Box<Task>` is provably never dropped while its handle lives. Stale-ref writer: `get_task_stack_pointer_ref` (task_manager.rs:76-81) → `state.rs:31/:48` raw `*mut usize` consumed by asm at `state.rs:35/:56`; both sites name `self.scheduler` or `current_task`, live-by-construction, no `remove_task` between ref-take and use; E2 (byte-identical corpse pre/post FIX-E) already refuted it as the active vector. **No new concrete defect in target 3.**

## 4. Cross-check vs R8 survivors + discriminating observations

- **A1 ⇒ H1 (allocate-side double-handout, boot-era/small-adjacent variant)** — the only surviving mechanism that produces a double handout with BAD-FREE=0 and that R8's gate structurally could not see (A2). H2 (out-of-band writer): targets all audited here — ELF in-bounds (§2), `initialize_stack` frame 16 words inside `[top−0x88, top)` (`arch/x86_64/src/cpu.rs:76-115`, in-bounds for the 16 KiB array), stack overflow writes non-zero (crime scene is zero) — H2 demoted. Stale-reference writer: §3 — dead.
- Discriminator per candidate: **A1** — one QEMU run of the §R2.2 command with the R8 gate retrofitted per A2 (seed table from `alloc_head` walk; print `R8-FREECONTAM` once if `insert_free_block`'s range intersects a live alloc-list range — today that line is *unreachable by design*, which is itself the blindness proof). **A3** — zero runs: grep existing logs for overlapping `[MEMORY] Region:` lines. **ELF/reloc (H2 residue)** — zero runs: host test below.
- Unit-test proposals (host, no QEMU, deterministic):
  - **U1 invariant fuzz:** single region, LCG-driven alloc/dealloc sequence mixing debug-observed sizes (`8..0x200`, `0x4030` Task-box, `0x1000+` image sizes); after every op recompute live ranges (walk `alloc_head`) and free entries (walk `head`) and assert alloc×alloc, free×free **and alloc×free** pairwise disjointness + total coverage. Convicts the allocate/coalesce engine as self-inflicter if any violation; 10⁶-op green denies it. (#59 tests never assert alloc×free cross-disjointness.)
  - **U2 A1 demonstration:** alloc A,B; `deallocate(ptr_B)` (models any party naming a foreign live block); a subsequent `allocate` of B's size returns a pointer inside B's old range while B's bytes are expected live — machine-countable proof the current admission model releases live memory; after the §5 fix the same script must flip to a `BAD_FREE_COUNT + 1` rejection.
  - **U3 adversarial ELF bounds:** craft ELF64/32 fixtures with `p_filesz > p_memsz`, segment `p_vaddr ≠ 0`, bss tail, dyn-not-in-load; assert `load_elf` result is `Err` or caught-panic and the image content equals the exact expected byte map — convicts/denies §2's relocation-read residue cheaply.


- **U1 invariant fuzz:** single region, LCG-driven alloc/dealloc sequence mixing debug-observed sizes (`8..0x200`, `0x4030` Task-box, `0x1000+` image sizes); after every op recompute live ranges (walk `alloc_head`) and free entries (walk `head`) and assert alloc×alloc, free×free **and alloc×free** pairwise disjointness + total coverage. Convicts the allocate/coalesce engine as self-inflicter if any violation; 10^6-op green denies it. (#59 tests never assert alloc×free cross-disjointness.)
- **U2 A1 demonstration:** alloc A,B; `deallocate(ptr_B)` (models any party naming a foreign live block); a subsequent `allocate` of B's size returns a pointer inside B's old range while B's bytes are expected live — machine-countable proof the current admission model releases live memory; after the section 5 fix the same script must flip to a `BAD_FREE_COUNT + 1` rejection.
- **U3 adversarial ELF bounds:** craft ELF64/32 fixtures with `p_filesz > p_memsz`, segment `p_vaddr != 0`, bss tail, dyn-not-in-load; assert `load_elf` result is `Err` or caught-panic and the image content equals the exact expected byte map — convicts/denies section 2's relocation-read residue cheaply.

## 5. Verdict — top suspect and the one first change

**Top suspect: A1** — a validated-but-alloc-blind free-list insert (probe vs free entries only, `:198-207`; identity-only dealloc walk, `:143-152`; user-namable heap pointers, `syscall.rs:69/:73`) releasing a range that still belongs to a live boot-era `Box<Task>` (idle), then zero-filled by the next legitimate bulk owner. It is the only candidate consistent with all standing facts: deterministic debug-only, BAD-FREE=0, zeroed frame with plausible SP, and R8 gate silence explained by A2 rather than innocence.

**First change (one thing only, kernel-only):** in `insert_free_block`, extend the admission probe to also walk the **alloc list** and reject (via existing `bad_free("overlaps-live-alloc", ...)`, no mutation) any inserted range intersecting a live block's `[start, start+(*allocHeader).size)`. Zero false-positive risk (a free range must never touch a live block by definition); the fix doubles as the witness — the very next existing repro run either prints the contamination line (conviction, mechanism-named, size-logged) or falsifies the whole insert-contamination class at the cost of one run, promoting A3/U1 follow-up. Acceptance: `cargo test -p kernel -- --test-threads=1` (orchestrator baseline; prior R8-era bar was 192 + new U-tests), `cargo xtask build` and `--debug`, `cargo build -p rosx-x86` from `arch/x86_32/` (`memory/**` is shared), then R2.2 greps on both games; warning posture delta-new=0 vs the recorded fingerprint (both `warning:` and `warning[EXXXX]:` forms).

# R10 — Reconciling Run-2.2 watchpoint evidence with the witness ledger

## 1. F1+F3 vs F5 — models checked against current source (`34f17b7`)

Source facts first (re-read this turn; they bound every model):

- **Restore is a live field read.** `switch_to_task` (`state.rs:23-26`) → `get_task_stack_pointer` → `Ok(task) => task.stack_pointer()` / `Err(_) => 0` (`task_manager.rs:69-74`). No cached pointer, no `ExecutionState` copy: the poison `0x13b10` was in slot 1's `Task` field at switch time.
- **Asm contract.** `swap_context` (`context_switching.S`): 15 pushes then `mov [rdi], rsp` (:22) — the 16-word frame is `[stored, stored+0x80)`, RIP word = `stored+0x78`; `mov rsp, rsi` (:24), 15 pops, `ret` (:41) exits at `stored+0x80`. The SP math happens on the **currently executing stack**, and `rdi` is whatever raw ref Rust passed: `get_task_stack_pointer_ref` → `pub fn stack_pointer_mut(&mut self) -> *mut usize` (`task.rs:89-91`) — a `*mut` into the arena box, taken pre-swap, written during the switch. `0x13b10+0x78 = 0x13b88` (watched slot), `+0x80 = 0x13b90` (corpse RSP) — exact.
- **`prepare_entry` is the only −0x80 stepper.** `initialize_stack` (`cpu.rs:68-117`) aligns in `sp` down (`(sp_val & !0xF)`), writes 16 words, returns `aligned−0x80`. So top `0x13d98 → 0x13d10` (F5's 0x88 = align-8 + frame-0x80 ✓), and each repeated call steps exactly −0x80: `0x13c90, 0x13c10, 0x13b90, 0x13b10` = **F5's descent chain, 4 steps to the poison value**. Each call also writes sentinels `0x0f…0x15` + entry_point into the new frame — **including a nonzero word at `sp+0x78`**, i.e. a step landing at `0x13b10` would have written `0x13b88`. But shipped call sites prepare idle exactly once (`kernel.rs:164`; the others are `:130` scheduler task, `:289` per-`schedule()` fresh task). A save cannot mimic it either: idle's live RSP measured `0x13d48`; an hlt loop cannot save 0x238 deeper.
- **Slot-1 identity is API-stable.** Arena item = `Option<Box<Task>>`; `add` takes a free slot gen-current; `remove` bumps gen (`arena.rs:65` — F5 gen=0 ⇒ never removed); `replace` (:70-77, keeps gen, drops old box) has exactly one production caller, `future.rs:157`, on the **future** arena — never `TaskManager`. `KernelServices` is `Once`-init ⇒ one TaskManager. `scheduler.rs:198` `self.idle_task = Some(returned_handle);` is the equal-branch self-assignment — idle is never re-registered.

Model-by-model:

- **(a) two objects/stacks behind handle 1.0 — SURVIVES, refined.** API-duplicate registration is dead (above). The surviving form: a **second owner of idle's 16 KiB array** — another construction/execution treating those bytes as its own while slot 1 keeps its name `"[K] Idle"` and its box. F3 is the live evidence: `wrap_index::<kernel>` (scheduler-queue code) spilling at `RSP+0x20 = 0x13b10` with `RSP = 0x13af0` — a running context's stack **inside** idle's array. The field could then legitimately receive the other context's RSP through the asm `mov [rdi], rsp` (ref = idle's field, live SP = the other owner's stack) — a save whose SP math runs on the wrong stack, matching the delegation's phrasing precisely.
- **(b) descent ledger ≠ restore target — OPEN, ledger blind spot.** The field writes were handle-scoped; **no prior witness ever printed the box address**, so "consistent through setters" never proved object identity. And per the source above, 4 extra `prepare_entry` calls on idle are unreachable in shipped code — so the ledger's descent was written by something else stepping −0x80, or the materialization attestation and the poison window belonged to different physical objects. Flag, don't assume.
- **(c) box moved without gen bump — REFUTED as stated.** Post-#58 `replace` keeps gen but TaskManager never calls it (grep); `remove` bumps gen and gen stayed 0. The only non-API variant is heap-level (bytes under a stable box re-handed out) — that is model (a) refined, not an arena event.
- **(d) L2-vs-L3 tension — RESOLVED as a report-priority artifact, not a new fact.** Both watchpoints were conditioned `stop only if *(unsigned long*)0x20000013b88 == 0`; nonzero stores at b88 fire but are suppressed and uncoun ted. L2 (b88-only) reported the boot-era Task-construction zero-copy (RIP `0x2c21ec` = `compiler_builtins` memcpy `copy_forward` — exactly what `Box::new(Task{ stack:[0;2048], … })` lowers to) at b88; L3 (both armed) reported the same class of wide event once at the lower address (wp2 stop #1, Old `<unreadable>`, boot era), per the report's own priority caveat. Consequence, and this is the load-bearing reconciliation: **b88's 0 at death is boot-original zero-init** — no nonzero ever landed there post-attach (a nonzero save landing + later zeroing would have been caught; a nonzero landing alone contradicts the corpse). The `[0x13b10, 0x13b90)` window never received a frame for that chain, while F5's chain reached it — the chain's writes and the window's bytes belong to different physical ownership. That is F1+F3 vs F5 reconciled: the ledger is true handle-side, false object-side; F3's foreign RSP and F2's small-int heap stores occupy what the ledger believed was idle's deep stack.
- **F6 fits:** Run-2.1's guard probed `insert_free_block` against `alloc_head` only; a double-handout created **allocate-side** (split/first-fit) or by **region-seed overlap** (`new()`, A3) never passes that probe. Its silence is compatible with (a)-refined, innocent only for the insert-contamination class.

## 2. Release-greenss

(a)-refined explains it without the zero frame existing: whether idle's array is shared (and where the other owner's stale SP lands) is a pure allocation-layout fact. Release layout shifts the second owner away (or makes the other owner's SP land on a materialized frame) — poison-SP class: debug vs release differ in **where the stale sp points**, not in whether the sharing/aliasing happened. F4's dual determinism (tick 2821→979 across instrumented builds, corpse address byte-invariant across 4 families) is layout-function behavior — reproduced, not explained, by any call-path model; (b)'s "extra prepare" would need a build-varying call count, and no such branch exists in source — another pressure point toward (a).

## 3. Single next scratch experiment (RUN-W3, one snake run)

Rev-4/5 discipline: `#[cfg(debug_assertions)]`, `RW3` markers everywhere, save `/tmp/opencode/rosx_pre_r10.diff` first, byte-equal revert attestation after. Kernel-only, zero bytes in `arch/`, no D1/D2/FIX-E re-import. One datum is missing from all prior evidence: **Box<Task> address at each witness event**. Sites (prose contracts):

1. `Task::new` return: `RW3-NEW name=<s> box=0x<hex> top=0x<hex>` (box = `&*task` as usize; top = `stack.as_mut_ptr().add(len)`). All tasks: ~10 lines total.
2. `TaskManager::add_task`/`remove_task` success paths: `RW3-ADD idx=<n> gen=<n> box=0x<hex>` / `RW3-DEL idx gen box`. An `RW3-DEL idx=1` line is itself a conviction of model-(c) revival.
3. `prepare_entry` after `set_stack_pointer`: `RW3-PREP idx gen box sp frame_or call_n` (W-a plus box).
4. `switch_to_task` pre-`swap_context`: `RW3-LOAD idx gen box sp sched_box saveptr_off` — incoming handle's box, incoming sp (the field value), scheduler box address, and `(scheduler_stack_pointer_pointer − sched_box as *mut usize)`: proves which object the save slot lives in.
5. `switch_to_scheduler` pre-`swap_context`: `RW3-SAVE idx gen box spref_off rdi_is_box_field=<bool>` for the outgoing handle.
6. Invariant check at both sites (SP-range + zero-frame soft rule as R4): on any violation, print `RW3-CONVICT …` with both boxes, both ranges, sp, and the saved ring.

Output budget: idx∈{0,1} switch lines + all NEW/ADD/DEL/PREP; hard cap 2048 lines with a single `RW3-CAPPED` sentinel (death at tick ~979 bounds the whole game window; Run-2.2 `stdout_full.log` already survived full board frames — volume is safe). Run: `KERNEL=/home/ghisi/git/rosx/target/rosx/debug/rosx PHASES="wait:35" timeout 110s python3 -u /tmp/opencode/repro_triple_fault.py snake 2>&1 | tee /tmp/opencode/rw3_snake.log`. Build gates (all proven-green forms on this base per Run-2.1 G2/§1): `cargo test -p kernel -- --test-threads=1` (= 220), `cargo xtask build --debug`, `cargo xtask build` (release-parity: witness must vanish with `debug_assertions`), Δ-new=0 vs the untouched-`34f17b7` fingerprint {kernel 60, rosx bin 26, shell 3, +1 manifest} both `warning:` and `warning[EXXXX]:` forms.

Conviction tests (POSIX greps on the log): **T1** `RW3-LOAD idx=1` box values — set size >1, or restore-time box ≠ last `RW3-SAVE idx=1`/`RW3-PREP` box ⇒ object-swap convicted. **T2** any two `RW3-NEW`/`RW3-ADD` printed `[box, box+size_of::<Task>)` windows overlapping ⇒ double-handout convicted (machine-check from printed hex ranges). **T3** all boxes distinct, no overlap, yet `RW3-LOAD idx=1` sp outside its own `[base, top)` ⇒ planted via raw-ref/aliasing.

## 4. Falsifiable table

| Model | Expected RW3 signature | Falsifier |
|---|---|---|
| (a-refined) second owner of idle's array | T2 hit: another `RW3-NEW` window ∩ idle `[base,top)`; `RW3-SAVE idx=1` present with `spref_off` = stack_pointer field offset while LOAD-side shows foreign stack | all windows pairwise disjoint AND T1,T3 silent |
| (b) ledger misattribution | exactly one box per idx forever; T3 silent; zero-frame conviction shows sp inside the box's own array with sentinels absent ⇒ the descent record itself was wrong about physical target | descent reproducible on one box address with matching `frame_or` at death sp |
| (c') arena/box swap | T1 hit (≥2 boxes for idx 1) or any `RW3-DEL idx=1` | single stable box for idx 1 all run |
| (d) artifact-only (no object mystery) | n/a — already resolved; predicts nothing new | a post-attach conditioned stop at b88 in any future armed rerun |
| allocate-side double-handout vs A3 regions | T2 with both boxes allocated post-`new()` ⇒ allocate-side; overlap equals two `[MEMORY] Region:` lines ⇒ A3 (zero-run check on existing logs) | T2 silent with disjoint prints ⇒ (a) dead, escalate T3/aliasing |

## 5. Ranked bet + fix directions (sketch only, one line each)

**Bet: (a-refined)** — handle 1.0's `Box<Task>` is stable but its 16 KiB array has a second live owner acquired allocate-side (invisible to Run-2.1's insert-only guard and to R8's late-table gate per A2); the field's poison is a legitimate `mov [rdi], rsp` write taken while `current_task` named idle and the executing SP was the other owner's; F5's descent is handle-true, object-false. If RUN-W3 convicts:

- T2/allocate-side: range-disjoint admission on the **handout** path mirroring #59's insert probe + multi-region host fuzz — kernel-only.
- T2/A3: reject-overlap region seeding in `new()` from the multiboot map — kernel-only.
- T3/raw-ref: stage the save through a fixed slot (the parked FIX-E idea), so no raw `*mut usize` into an arena box survives the call boundary — kernel-only.
- T1/swap: re-audit `TaskManager` registration paths; do not touch arena gen semantics without a new plan.
- Silent + conviction-empty: stop-and-report per D4; no speculative fix, no arch edit (USER-GATE stands).


## RUN-W3 RESULT (orchestrator summary 2026-10-13; full detail in artifact "Run-2.3")

- R10 §4 row (b) CONFIRMED: ledger-misattribution class. Poison sp=0x13b10 caught live inside handle 1.0's CORRECTLY-OWNED, never-swapped box (0x2000000fd70); frame at sp: 16/16 words zero (frame_or=0 — the descent is NOT reproducible on the box); prepare_entry never re-run (call_n=1); scheduler save-slot healthy at fatal switch (saveptr_off=0x10). (a-refined), (c'), allocate-side/A3 all refuted by their own falsifiers (T1 single stable box; T2 zero window overlaps; T3 sp inside own [base,top)).
- OPEN QUESTION to architect: the writer of the −0x80×4 descent into the correctly-owned stack_pointer FIELD is unidentified — ring-based exclusion says not-the-legitimate-switch-save path; CAVEAT for R11: verify ring depth from the artifact hook list — if shallow (≤32), the exclusion is tail-only, NOT whole-run, and boot-era/spawn-burst saves are unattested.
- QEMU launches this session so far: 7 (2 baseline re-verify, L1-L3 watchpoints, W3 ×2 incl. disclosed self-livelock retry). Tree clean at 34f17b7; scratch fully reverted with attestations; no commits.

# R11 — Writer enumeration + decisive experiment (read from current source at `34f17b7`; read-only turn, no files written, no commands run)

## 1. Every write path to the field word `idle_box + 0x10` (`Task::stack_pointer`)

**(i) Rust setters.** Complete grep this turn — `set_stack_pointer` (task.rs:92) has exactly two callers: `Task::new` (task.rs:78, value = stack top after literal `stack_pointer: 0` at :69 — once per box, pre-registration) and `Task::prepare_entry` (task.rs:147). `prepare_entry` calls `cpu.initialize_stack(self.stack_pointer(), …)` (task.rs:141) — the tree's only arithmetic −0x80 stepper (cpu.rs:80 `sp = (sp_val & !0xF)`; 16 writes :82-113; returns `aligned−0x80`) — and it also **materializes 16 nonzero-bearing words at every step it takes**. Call sites: kernel.rs:130 (Main, once), kernel.rs:164 (idle, once — `Kernel::setup`), kernel.rs:289 (`schedule_task`, fresh tasks only). W3 hook (inside `prepare_entry`) printed `PREP idx=1 … call_n=1`: **idle-prepare-refuted**; `Task::new`-to-this-box writes only `0` and `0x13d98`.

**(ii) asm `mov [rdi], rsp`** (context_switching.S:22, after 15 pushes :7-21; `test rdi, rdi`/`jz skip_save` :5-6). Reaching `rdi`, exhaustively:
- **switch_to_task** (state.rs:35): `rdi = get_task_stack_pointer_ref(self.scheduler)` (state.rs:28-31) → writes **Main idx0 box + 0x10, never idle's** (`scheduler` handle fixed at kernel.rs:139).
- **switch_to_scheduler** (state.rs:56): `rdi = get_task_stack_pointer_ref(current_task)` (state.rs:45-48) → **idle's field is written iff `current_task = Some(idle)`**; value = executing-rsp-after-call−0x78. Entry points: `wait_future` (kernel.rs:229), `task_yield` (:246), `preempt` (:254, timer-ISR context, interrupts.rs:112), `terminate_and_yield` (:275). Pairing anomalies where `current_task` names idle while executing elsewhere: state.rs:27 sets `current_task` *before* the swap at :35 (nested tick there → Main-stack rsp, window-off value); switch_to_scheduler's `sti` (asm:40)→`ret` (:41) window re-enables IF mid-restore (nested tick there → save base = loaded_sp − 0x28 − H, a fixed step, H = handler+preempt+switch frame; kernel never IST-relocates — ticks run on the interrupted stack).
- **Boot** (kernel.rs:177-178): `rdi = null` → `jz skip_save`, no store.
- **Syscall path**: syscall.S pushes/pops only (IF masked, cpu.rs:52); switches reach it only via `wait_future` above. No field store of its own.

**Frame theorem (the step-size constraint, load-bearing):** the store is *preceded* by 15 pushes plus the call's return-address push ⇒ **any asm save at base B writes 16 words `[B, B+0x80)` including a non-zero kernel code address at `B+0x78`** (return into the `swap_context` wrapper, cpu.rs:120-130). wp1 (`0x13b88` = 0x13b10+0x78) recorded **zero** true stores, and Run-2.3 caught `frame_or=0` ⇒ *the final write of 0x13b10 was not itself an asm save on that stack*. Intermediate steps (0x13c90/0x13c10/0x13b90) were never watched, so a save-then-bulk-erase chain is not excluded — and Run-2.2 empirically proves bulk `rep movsq` zeroing walks exactly this window (stops #1/#3) plus a context with **live rsp=0x13af0 inside idle's array** (stop #2, `wrap_index` spill): idle's deep window is *executed and heap-written by someone* while the field descends into it.

**(iii) Raw refs outliving a call.** `stack_pointer_mut` (task.rs:89-91) `pub`; its only consumer is `get_task_stack_pointer_ref` (task_manager.rs:76-81), itself called at exactly two places (state.rs:31, state.rs:48), each consumed by the immediately following `swap_context`. No stored/cached `*mut usize` exists in the current tree (grep-proven this turn; the FIX-E staging that could stage-and-defer is reverted) ⇒ stale-ref-from-a-previous-switch has **no holder** — it survives only as heap-level aliasing of the box address, which W3's T1/T2 constrained to stable-box, task-window-disjoint.

**Ranking vs (A) exact −0x80×4, (B) ring survival, (C) debug-only-leafing.** Ring depth from the Run-2.3 hook list = **32** ("32-entry ring"); a 32-slot ring holding 2 entries at death is self-contradictory (boot+shell+spawn ran ≫2 switch events) ⇒ print/wrap is unreliable ⇒ **exclusion is tail-only; boot/spawn-era saves are unattested**.
1. **Asm save, switch_to_scheduler, `current_task=Some(idle)`, executing on a stack inside idle's window, frames later bulk-zeroed** — only writer class that produces idle-window values; step = 0x28+H per nesting event (constant under constant handler chain — matches a constant −0x80 if H=0x58, checkable live); survives (B) tail-only; (C) via layout-dependent eraser. **Rank 1.**
2. **Nested `sti`/`ret`-window variant of #1** (the R2-H1 window, survived R3a because R3a's one-line fix was reverted with the D1/D2/FIX-E bundle) — same machinery, distinct RIP capture; rank 2.
3. **prepare_entry repeat** — refuted (call_n=1) *unless* box identity lied; T1 says it didn't. Rank 3 (excluded).
4. **Bulk copy carrying a stale field word** (rep movsq class) — needs a source already holding 0x13b10, so always downstream of #1/#2. Rank 4.
5. **Task::new / raw-ref holder** — dead structurally.

## 2. No commit-gated repo change this round

Experiment runs on the **pristine** tree: `cargo xtask build --debug` (green proven on this base, Run-2.1 G2), zero scratch — the whole point of the ELF walk is that instrumentation is what poisoned the ring's reliability. `git status --porcelain` must stay at its pre-existing single entry.

## 3. Decisive experiment (adopt-with-replacements; budget ≤ 2 launches)

**Replace W3-lite scratch with the static ELF walk.** Box addresses were **never compared across builds** (only `0x…fd70` within the Run-2.3 v1/v2 family; the 4-build determinism datum is the *corpse SP*, not the box) ⇒ do not transplant `0x2000000fd70`. Feasibility: ELF is **EXEC, vaddrs == runtime** (Run-2.2 L3), binutils 2.46 + gdb 17.1 present (Run-2.2 §2), symbol chain exists: `kernel_services::KERNEL_SERVICES` (`Once<KernelServices>`, kernel_services.rs:79) → `task_manager: &'static KernelCell<TaskManager>` (:70) → `TaskManager.tasks` arena → `items[1]` = `Option<Box<Task>>` niche word (stride 8; coder takes exact field offsets from DWARF/`objdump -g`). **Mandatory zero-launch pre-flight:** resolve the address offline in `gdb --batch target/rosx/debug/rosx` (and/or python); only then launch.
- **Launch 1:** `wd_repro.py` clone (exists from Run-2.2; re-clone from `repro_triple_fault.py` if gone) + `-accel tcg -S -s`, `PHASES="wait:35"`, `timeout 120s`, snake. gdb attaches **at reset** (solves rose>-invisibility); arms **unconditioned** `watch *(unsigned long*)$ADDR` first: expect exactly 3 boot stops at the address (values 0 → 0x13d98 → 0x13d10) and `x/s *($ADDR-0x10)` reading `[K] Idle` — machine self-test that the watched word is the field. Then set `condition 1 $lr >= 0x20000013000 && $lr < 0x20000013d00` (post-write value; suppresses construction, idle PREP 0x13d10, legit saves ~0x13d48). Capture per stop: RIP + `x/i $rip` + `info registers` + `x/16gx $rsp` + **`x/16gx <new_value>`** + `(new − $rsp)`; ≤16 captures then latch-and-continue. Symbolize post-hoc against the same debug ELF. Expect ≥1 stop before death; 4 stops at 0x13c90/0x13c10/0x13b90/0x13b10 with RIP in `swap_context` + materialized frames ⇒ rank-1 convicted with mechanism named; `copy_forward` RIP ⇒ rank-4.
- **Fallback (budget Launch 2 only):** if inferior-call conditioning misbehaves, unconditioned watch + script-side bucket classifier (idle-window / Main-window-catches pairing anomaly / auto-continue) — per Run-2.2 lesson, read stops from the raw console channel, not `to_string`.
- **NULL result (self-test passed, 0 conditioned stops, yet death):** the field provably cannot reach 0x13b10 without a trapped store (restore reads it fresh — E4; guest never touches DR0-DR7 — Run-2.2 §3 audit; kernel identity-mapped) ⇒ NULL means **gdbstub trap loss** (watchpoint evicted during the exception storm), not writer invisibility → rerun Launch 2 unconditioned. NULL with self-test *failing* = walk wrong; no conclusion either way.

## 4. Certainty verdict

**The enumeration does NOT convict with certainty** — rank-1 survives but its step arithmetic (H=0x58) and its erasure companion are unmeasured, and the field word itself has been watched in zero prior runs (all watches hit stack words, never `box+0x10`). Therefore **no fix is contracted this turn**: run §3 first. If Launch 1 convicts rank-1/2, the repair direction is already parked and kernel-only: stop letting a raw `*mut usize` into the arena box drive the asm store — stage the save through a fixed `ExecutionState` slot and write back post-switch via the setter (FIX-E idea, `plans/scheduler-lifecycle-hardening.*`); USER-GATE arch/ stands untouched, parked bundle stays parked, nothing in this plan edits `arch/`.

# R12 — Fix contract: eliminate the debug stack temporary in `Task::new`

**Basis:** orchestrator-verified mechanism (R12 delegation): debug `Task::new` prologue `sub $0x180d8, %rsp` at `0x288050` ≈ 96 KiB stack temporaries, materialized via `rep movsq` (`copy_forward` 0x2c21e9/0x2c21ec = Run-2.2 watchpoint winner). Worktree holds Run-2.7's uncommitted `6144` arrays (`task.rs:55`, `:72`); HEAD `34f17b7` is `2048`.

## §1 Fact ledger under the mechanism — every open fact, accounted

- **(a) −0x80×4 descent of `stack_pointer` at `box+0x10` (0x2000000fd80).** FITS as splatter/alias, not a legitimate writer — consistent with W3's ring exclusion and `call_n=1`. Concrete identity: F3's underflowed spawner frame had `RSP=0x13af0` with spill slot `[RSP+0x20] = 0x13b10` — i.e. the poison sp is *exactly a word inside a `Task::new`-era frame region* of a spawn-burst caller (shell + snake + tetris, `schedule_task` inlines `new_elf_task`→`Task::new`). Successive burst calls sit one frame-step apart (constant call-chain ⇒ constant −0x80), each splatter depositing its own underflowed-RSP-shaped, plausible idle-window word onto the aliased field slot; the temp aliasing itself across calls is the self-consistent source (the memcpy source region = caller stack bytes previously splattered). **LOUD GAP:** the exact −0x80 step size is not arithmetically pinned by existing artifacts; §2 step 2's objdump/frame walk is the arbiter, and the post-fix gate (descent class cannot recur — no temps exist) is its falsifier. No fact contradicts.
- **(b) `frame_or=0` at poison sp, `call_n=1`.** FITS exactly: the bulk `rep movsq`/zero-fill splatter is the in-tree bulk zero writer (the delegation's own wording: temps are "zero-containing"); no `prepare_entry` ever ran there (`call_n=1`), and R11's frame theorem (an asm save always writes nonzero at `B+0x78`) is preserved — the window was zeroed by splatter, never saved.
- **(c) RUN-W3 T1/T2 silent (stable boxes, registered windows disjoint).** FITS: registration covered task stack windows derived from `Task` fields; temporaries live on the *caller's* stack and reach across the heap unregistered — pairwise window checks structurally cannot see them.
- **(d) F3 foreign live RSP 0x13af0 in victim window.** FITS and is load-bearing: a 16 KiB task stack minus a 0x180d8 temp frame puts the spawner's RSP ~80 KiB below its own stack bottom, i.e. inside idle's array — "the deep window is executed and heap-written by someone" is the spawner itself, mid-splatter.
- **(e) Determinism + debug-only + Run-2.7 corpses-moved.** FITS: fixed boot/spawn order + fixed debug codegen sizes ⇒ fixed splatter geometry; release elides temps (delegation fact: release green always); 3× stack resized the splatter and moved the corpse — the layout-sensitivity datum this mechanism predicted and allocator/ISR theories never produced.
- **(f) Setter-path witnesses blind.** FITS: splatter = raw compiler stores + memcpy; every prior witness (`DRIFT`, shadows, setters) convicts only Rust writes; field-word watchpoints were never armed (R11 §4).
- **(g) 48 KiB boot death.** FITS: Run-2.7's `6144` array ⇒ `Task` value = 0xC030 ≈ 48 KiB; first `Task::new` (Main Thread, boot context) needs ~288 KiB temps; bootloader-provided stack underflows ⇒ CR2=0xffffffca40, RSP≈0. Boot survives at HEAD only because ~96 KiB still lands in mapped memory pre-heap-contention; exact boot-stack size is not derivable from repo source (bootloader crate owns it) — marked non-derivable, not contradictory.

**Verdict: no fact contradicts the mechanism; one arithmetic gap ((a) step size) is assigned to a gate, not waved.**

## §2 The fix — ONE change, kernel-only

- **Step 0 (revert).** `git restore kernel/src/task.rs`; gate: `git diff --quiet HEAD -- kernel/src/task.rs` silent AND `grep -Ec '6144' kernel/src/task.rs` → `0`. (Restores `stack: [usize; 2048]` exactly — the array size is frozen; tuning it is *not* a mitigation.)
- **Step 1 (contract).** File `kernel/src/task.rs`, body of `Task::new` only (signature `pub fn new(name: &'static str, entry_point: usize, entry_param: usize) -> SharedTask` unchanged; return type `Box<Task>` unchanged; no caller edits). Replace `let mut task = Box::new(Task { … })` + the trailing unsafe `set_stack_pointer` block (task.rs:65–79 current) with: allocate `Box<MaybeUninit<Task>>` via `MaybeUninit::uninit()`; take `*mut Task` with `Box::as_mut_ptr`; inside ONE `unsafe` block write every field exactly once via `core::ptr::addr_of_mut!`+`ptr::write` — `name`, `state: Created`, `yield_reason: None`, `stack_pointer: <final top>`, `entry_point`, `entry_param`, `completion_future: None` — and the array with `ptr::write_bytes(stack_ptr, 0, 2048)`, where the final top is `stack_ptr.add(2048).addr()` (same value as today's `as_mut_ptr().add(len).addr()`); return via `Box::assume_init()` (stable since 1.60 — repo is nightly). Required imports: `core::mem::MaybeUninit`, `core::ptr::{addr_of_mut, write_bytes}`. No comments.
- **Why form (1):** option (2) `Box::new_zeroed` is **rejected** — `name: &'static str` is not zero-valid (null fat ptr), and `Option<FutureHandle>`'s zero repr is unspecified — even though every *other* field (`TaskState`, `Option<YieldReason>`, 3×`usize`) is zero-valid. Option (3) restructure is a bigger diff than required. Field-completeness proof: the eight fields are enumerable verbatim from `task.rs:49–56`; **drop-safety is trivial** — every field is `Copy`/POD (`handle.rs:85` `#[derive(Copy, Clone, …)] pub struct FutureHandle(Handle)`; `task.rs:12/18` enums `Copy`; `Task` has no `Drop`), so a partial `MaybeUninit` can leak nothing; and the kernel has no unwinding anyway — `handle_panic` prints `!!! PANIC !!!` and `loop {}`s (panic.rs, per R4 §0).
- **Step 2 (scope walk — the 96 KiB mystery).** Source-level enumeration (my grep this turn): the only ≥8 KiB aggregate in `kernel/src` is `task.rs` `stack`; `vec![0u8; …]` sites are heap `Vec`s (tests + `elf/mod.rs:85/:140` — heap, unaffected); `initialize_stack` writes 16 words. So all ~96 KiB is repeated debug copies of the single 0x4030-byte `Task` literal (6×0x4030 = 0x18120 vs prologue 0x180d8: consistent with slot sharing/alignment; one fix site covers it because the literal is the only value-side construction). Machine arbiter (binutils 2.46 confirmed present, Run-2.2 §2): `objdump -d target/rosx/debug/rosx | grep -E 'sub +\$0x[0-9a-f]{4,},.*rsp'` → **post-fix: zero hits attributed to `Task::new`/`schedule_task`/`new_elf_task` region; pre-fix hit at `0x288050`-equivalent gone.** If the walk names any *other* ≥8K temp, STOP and report — scope of a second change is an orchestrator decision, not the coder's.

## §3 Acceptance gates (order; commands verbatim)

1. Revert gate: step-0 checks above (`git diff --quiet HEAD -- kernel/src/task.rs`; no `6144`; `grep -Ec 'stack: \[usize; 2048\]' kernel/src/task.rs` == 1 — re-checked on the final tree).
2. `cargo test -p kernel -- --test-threads=1` → **221 passed** (220 baseline, delegation/R10 §3 transcript-sourced, **+1 new test** `task_new_field_integrity`: `Task::new("t", 0x1111, 0x2222)` in `task.rs mod tests`, assert `name()`, `entry_point()`, `entry_param()`, `state()==Created`, `yield_reason().is_none()`, `completion_future().is_none()`, `stack_pointer()` nonzero, and `task.stack.iter().all(|w| *w == 0)` — same-module access is legal).
3. `cargo xtask build --debug` and `cargo xtask build` green; `objdump` scan of step 2 against the debug artifact; x86_32 parity `cargo build -p rosx-x86` from `arch/x86_32/`.
4. Warning posture **Δ-new = 0** vs delegation fingerprint {kernel lib 60, rosx bin 26, shell lib 3, +1 manifest}; counting filter `grep -Ec 'warning(\[[A-Z0-9]+\])?:'` (both forms); decreases at deleted sites are not violations.
5. §R2.2 both apps, sequential (`/tmp/opencode/repro_triple_fault.py`, `PHASES="wait:35"`, `timeout 110s`, `python3 -u`): `grep -c -E 'Triple fault' "$TMP/exc.log"` == **0 (binding)**; `!!! PANIC !!!` == 0; `v=0e`/`v=08` == 0; **markers now required:** snake log `game_over_snake: True`, `goodbye: True`; tetris frame-continuation through the horizon (`124` ⇒ one rerun +50s).
6. Remove the two `#[ignore = "red: deterministic triple fault…"]` attributes (`tests-integration/tests/app_lifecycle.rs:16`, `:58`), bodies untouched; then `cargo test -p tests-integration --test app_lifecycle -- --test-threads=1` → **2 passed**; `--test boot_and_shell` → **4 passed**; `--test ipc_random` → **1 passed**; `cargo xtask test --skip-integration` green; final `grep -E '#\[ignore' tests-integration/tests/app_lifecycle.rs` → empty.
7. **QEMU launch budget: 9 sequential boots** — structural enumeration: repro 2 + `app_lifecycle` 2 (`:19`,`:61`) + `boot_and_shell` **4** (four separate `QemuSession::spawn()` at `:6/:13/:20/:28`) + `ipc_random` 1 (`:83`). The delegation's ~7 undercounts boot_and_shell as <4; only counting the repro pair as substitutable for app_lifecycle would reach 7 — not recommended.

## §4 Hygiene (future plan only — NOT contracted here)

- The disassembly pattern class ("construct big value, bulk-move to heap") is suicidal under 16 KiB task stacks anywhere it reappears; today the spawn/elf path is clean post-fix (step-2 walk is the standing proof) — but any future `Box::new(<≥8K aggregate>)` reintroduces it. Future plan should carry an automated CI lint: the step-2 `objdump … grep 'sub +\$0x[0-9a-f]{4,},.*rsp'` scan as a machine-countable xtask check (no QEMU).
- Future debug-only stack guard: canary high-water check of each task's inline array at `prepare_entry` and on each preemption save; a violation names the splatterer before it kills a neighbor.
- Record: stack-capacity tuning (Run-2.7's 3×) is a mask, never a mitigation — it moved the corpse and killed boot.
- Orthogonal parked work untouched: allocator A1 insert-admission fix (R9 §5) and the D1/D2/FIX-E bundle stay parked; this fix requires neither.
