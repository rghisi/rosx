# Scheduler Refactor — Ports & Adapters (Hexagonal Architecture)

> **Status:** Done (2026-10-06)
> **Scope:** `kernel/src/scheduler/scheduler.rs` (+ its wiring in `scheduler/mod.rs`, `kernel_services.rs`, `kernel.rs`). No changes to strategies, task manager internals, syscall ABI, or arch crates.
> **Platform focus:** x86_64; `rosx-i686` shares `kernel/` and must keep compiling untouched.
> **Rule:** behavior-preserving reorganization only. One commit per step; verify + confirm commit message before proceeding (AGENTS.md).

---

## 1. Purpose

The first hexagonal pass on `Scheduler` injected six outbound ports (`ForSwitchingTaskContext`,
`ForCompletingExpiredTimers`, `ForReadingSystemTime`, `ForExpiringTimers`,
`ForHandlingHardwareInterrupts`, `ForManagingTasks` — defined in `kernel/src/lib.rs:36-85`) plus the
`SchedulingAlgorithm` strategy port, with Noop adapters (`scheduler.rs:16-46`) and test fakes.
Three structural violations remain, each with a concrete testability cost:

1. **The core loop is untestable.** `Scheduler::run()` (`scheduler.rs:95-122`) is an infinite
   `loop {}` bound to the `services()` global locator. The most important behavior in the
   subsystem — per-cycle ordering of interrupt handling, timer expiry, pick, switch, reconcile —
   is exercised nowhere; tests hand-reimplement the loop body instead (`run_one_round` at
   `scheduler.rs:548-552`; `expired_timer_futures_are_completed` at :726 and
   `hardware_interrupts_are_delegated_to_handler` at :769 re-code single steps inline, testing the
   fakes rather than the scheduler).
2. **Constructor telescoping + duplicated composition.** `new` / `new_with_context_switcher` /
   `new_full` (`scheduler.rs:61-93`) take 1/2/7 positional `&'static dyn` args, and
   `mfq_scheduler`/`fifo_scheduler` (`scheduler/mod.rs:17-39`) duplicate identical four-adapter
   wiring. Adding one port means touching every constructor and all ~38 call sites.
3. **Domain hardwires a production adapter.** `new` and `new_with_context_switcher` default the
   tasks port to `&crate::kernel::KERNEL_TASK_MANAGER` (`scheduler.rs:62, 70`) — domain reaching
   into infrastructure inverts the dependency direction. As a result every scheduler test calls
   `kernel_services::init()` and mutates the global task registry (`create_ready_task`,
   `scheduler.rs:326-331`), forcing the whole suite to run `--test-threads=1`
   (`xtask/src/main.rs:85`). The strategy tests (`mlfq_strategy.rs`, `fifo_strategy.rs`) use no
   `services()` at all — they are the hermetic model to match.

---

## 2. As-Is Snapshot

Verify against the tree before starting; line numbers drift.

- **`Scheduler` fields** (`scheduler.rs:48-58`): `algorithm: Box<dyn SchedulingAlgorithm + Send>`,
  `context_switcher`, `timer_handler`, `time_source`, `timer_expiry`, `interrupt_handler`,
  `tasks` (all `&'static dyn <port>`), `hw_interrupt_queue: VecDeque<HardwareInterrupt>`,
  `idle_task: Option<TaskHandle>`.
- **`run()`** (`scheduler.rs:95-122`): extracts the five adapter refs from `services().scheduler`
  once, then per iteration: `drain_hardware_interrupts` → `interrupt_handler.handle` →
  `timer_expiry.pop_expired(time_source.now())` → `timer_handler.complete_timer_future` per
  handle → `start_next_task` → `context_switcher.switch_to_task` →
  `reconcile_returned_task`. Never returns.
- **Constructors** (`scheduler.rs:61-93`): `new(algorithm)`;
  `new_with_context_switcher(algorithm, ctx, timer_handler)`;
  `new_full(algorithm, ctx, timer_handler, time_source, timer_expiry, interrupt_handler, tasks)`.
  The first two hardcode `&crate::kernel::KERNEL_TASK_MANAGER`; there is **no `NoopTasks`**.
- **Factories** (`scheduler/mod.rs:15-39`): `SchedulerFactory =
  fn(&'static dyn ForSwitchingTaskContext, &'static dyn ForCompletingExpiredTimers) ->
  Box<Scheduler>`; `mfq_scheduler`/`fifo_scheduler` both call `new_full` with
  `&KERNEL_TIME_SOURCE, &KERNEL_TIMER_EXPIRY, &KERNEL_INTERRUPT_HANDLER, &KERNEL_TASK_MANAGER`.
  Referenced by `KConfig.scheduler_factory` (`kconfig.rs:8`), `arch/x86_64/src/main.rs:43`,
  `arch/x86_32/src/main.rs:20`.
- **Kernel adapters** (`kernel.rs:32-101`): `KernelContextSwitcher` (private),
  `KERNEL_TIME_SOURCE`, `KERNEL_TIMER_EXPIRY`, `KERNEL_INTERRUPT_HANDLER`,
  `KERNEL_TASK_MANAGER` — these legitimately reach into `kernel()` / `services()`.
- **Composition**: `kernel_services.rs:85` builds a placeholder
  `Scheduler::new(FifoStrategy::new())`; `Kernel::new` (`kernel.rs:131-132`) calls the factory
  and `services().scheduler.replace(*scheduler)`. Other scheduler entry points from `kernel.rs`:
  `set_idle_task` :164, `push_hardware_interrupt` :205, `should_preempt` :249,
  `push_task` :295.
- **Test coupling** (`scheduler.rs:206-789`): `setup()` → `kernel_services::init()` (:219-221);
  `create_ready_task`/`create_running_task` write through `services().task_manager` (:326-331,
  :535-540); fakes exist for every port (`FakeAlgorithm`, `FakeContextSwitcher`, `FakeTimeSource`,
  `FakeExpiry`, `RecordingTimerHandler`, `RecordingInterruptHandler`) but none for
  `ForManagingTasks`.
- **Integration suite** (`tests-integration/tests/boot_and_shell.rs`): 4 QEMU tests
  (`kernel_boots`, `shell_banner_and_prompt`, `shell_echoes_keystrokes`,
  `shell_ls_and_unknown_command`) that boot the real kernel, inject keystrokes, and assert on
  guest output — they exercise exactly the paths this refactor touches (scheduler loop, keyboard
  interrupts, task wake). Included by `cargo xtask test` unless `--skip-integration`.

---

## 3. Target Shape

```
            inbound (unchanged)                domain                          outbound ports
  Kernel::main_thread_run ──▶ run() [thin driver]      ┌──────────────────────────────────────┐
  Kernel / syscall ──▶ push_task / wake_tasks ────────▶│ Scheduler::step()  (all logic)       │
                                                       │  uses ONLY self.ports + algorithm    │
                                                       └──────────────────────────────────────┘
                                                                 │ through
                                                       SchedulerPorts { context_switcher, timer_handler,
                                                         time_source, timer_expiry, interrupt_handler, tasks }
                                                                 │ implemented by
                                                       kernel adapters (KERNEL_*)  |  test fakes / Noops
```

Invariants preserved through every step:
1. Per-iteration semantics of the loop are **identical**: drain → handle interrupts → pop expired
   (at `now()`) → complete each → pick next (idle fallback sets `Running`, `unwrap` on missing
   idle kept) → switch → reconcile (Yielded non-idle: `Ready` + `record_yield` +
   `requeue_after_run`; Blocked: no-op; Terminated: `handle_termination`; Unchanged: no-op).
2. `SchedulerFactory` signature unchanged; `KConfig` and both `arch/*/main.rs` untouched.
3. No `services()` calls inside `Scheduler` after Step 1; no `crate::kernel::` adapter references
   inside `Scheduler` constructors after Step 3.
4. No behavior fixes smuggled in (see §7).

---

## 4. Execution Plan

> **Gate for every step (non-negotiable):** unit tests green, both kernels build, **and all 4
> `tests-integration` QEMU tests pass** before the step may be committed.

### Step 1 — Extract `step()` from `run()` (commit 1)
**Goal:** make one full scheduling cycle a pure method over injected ports; `run()` becomes the
boundary driver. This alone closes gap #1 and is testable with the fakes that already exist.
**Where:** `kernel/src/scheduler/scheduler.rs`.
**Do:**
- Add a private field `interrupt_drain_buf: Vec<HardwareInterrupt>` to `Scheduler` (replaces the
  loop-local scratch `Vec`, preserving today's allocate-once / `clear()` pattern).
- Add `fn step(&mut self)` containing exactly today's loop body:
  `drain_hardware_interrupts(&mut self.interrupt_drain_buf)` → `self.interrupt_handler.handle(..)`
  → `self.timer_expiry.pop_expired(self.time_source.now())` → complete each via
  `self.timer_handler` → `start_next_task()` → `self.context_switcher.switch_to_task(..)` →
  `reconcile_returned_task(..)` → clear the buffer.
- Rewrite `run()` as: `loop { services().scheduler.borrow_mut().step(); }`.
  No other line of `run()` survives; the up-front adapter extraction (:96-105) disappears since
  the ports are already fields.
- New tests driving `step()` on a `Scheduler` built from fakes:
  - `step_handles_interrupts_before_timer_expiry` (recording fakes assert call order);
  - `step_completes_expired_timer_futures` — replaces the hand-coded :726 test;
  - `step_routes_hardware_interrupts` — replaces the hand-coded :769 test;
  - `step_switches_to_picked_task_and_reconciles_yield` (multi-iteration: task yielded → requeued
    → picked again on the next `step`);
  - `step_falls_back_to_idle_when_nothing_ready`.
- Keep `run_one_round` tests (they exercise `start_next_task`/`reconcile_returned_task` directly —
  still valid), retire the two tests the new `step` tests subsume.
**Verify:** `cargo test -p kernel` ; `cargo build -p rosx` ; `cargo build -p rosx-i686` ;
`cargo test -p tests-integration -- --test-threads=1`.
**Commit:** `Extract testable step from scheduler run loop`.

### Step 2 — Bundle the six ports into `SchedulerPorts` (commit 2)
**Goal:** one constructor, one composition point; kills the 1/2/7-arg telescoping and the factory
duplication.
**Where:** `scheduler.rs` (struct + constructors, next to `Scheduler` and the Noop adapters),
`scheduler/mod.rs` (factories), `kernel_services.rs:85`, tests.
**Do:**
- `pub struct SchedulerPorts { context_switcher: &'static dyn ForSwitchingTaskContext,
  timer_handler: &'static dyn ForCompletingExpiredTimers, time_source: &'static dyn
  ForReadingSystemTime, timer_expiry: &'static dyn ForExpiringTimers, interrupt_handler:
  &'static dyn ForHandlingHardwareInterrupts, tasks: &'static dyn ForManagingTasks }` with a
  `noop()` constructor using the existing NOOP_* singletons (`tasks` temporarily also Noop — see
  Step 3) and a kernel composition helper `kernel_ports(ctx, timer) -> SchedulerPorts` built from
  the four `KERNEL_*` adapters (placed where the KERNEL_* types are visible).
- Replace `new`/`new_with_context_switcher`/`new_full` with `Scheduler::new(algorithm,
  SchedulerPorts)` (method visibility unchanged).
- `mfq_scheduler`/`fifo_scheduler` both become `Box::new(Scheduler::new(<strategy>::new(),
  kernel_ports(ctx, timer)))` — duplication gone.
- `kernel_services.rs:85` placeholder becomes `Scheduler::new(FifoStrategy::new(),
  SchedulerPorts::noop())` (it is replaced by `Kernel::new` before any use; confirm nothing
  touches it in between — `set_idle_task`/`push_task` all happen post-replace).
- Mechanical update of the ~38 test call sites; add `SchedulerPorts::with_ctx(...)`-style helpers
  only if call sites stay noisy.
**Verify:** `cargo test -p kernel`; `cargo build -p rosx`; `cargo build -p rosx-i686`;
`grep -rn "new_full\|new_with_context_switcher" kernel/` → empty;
`cargo test -p tests-integration -- --test-threads=1`.
**Commit:** `Bundle scheduler ports into a single dependency struct`.

### Step 3 — Invert the task-manager dependency; hermetic scheduler tests (commit 3)
**Goal:** the domain no longer defaults to any production adapter; scheduler unit tests run
without `kernel_services::init()`, like the strategy tests.
**Where:** `scheduler.rs` (add `NoopTasks` + test module rewrite).
**Do:**
- Add `struct NoopTasks` impl `ForManagingTasks` (get → `TaskState::Created`, set/remove no-op)
  and make it the `SchedulerPorts::noop()` `tasks` field — **removing
  `&crate::kernel::KERNEL_TASK_MANAGER` from `scheduler.rs` entirely**.
- Add test-only `FakeTaskManager: ForManagingTasks` over
  `Arc<Mutex<BTreeMap<TaskHandle, TaskState>>>` (+ recorded-call lists), in the tests module.
- Rewrite the test module: `create_ready_task`/`create_running_task` populate the fake instead of
  `services().task_manager`; drop every `setup()` call; keep assertions against the fake's state
  map. The `new_full`-based timer/interrupt tests rebuild `SchedulerPorts` from fakes.
- Leave `scheduler/mod.rs` factory tests as-is (they legitimately test the *composition* against
  real adapters).
**Verify:** `cargo test -p kernel`;
`grep -n "services()\|setup()\|KERNEL_TASK_MANAGER" kernel/src/scheduler/scheduler.rs` → empty;
`cargo build -p rosx`; `cargo build -p rosx-i686`;
`cargo test -p tests-integration -- --test-threads=1`.
**Commit:** `Decouple scheduler tests from global kernel services`.

---

## 5. Build & Test Commands

```bash
cargo test -p kernel                                    # fast; after every edit
cargo build -p rosx                                     # x86_64 no_std kernel
cargo build -p rosx-i686                                # x86_32 must keep compiling, untouched
cargo test -p tests-integration -- --test-threads=1     # QEMU gate: all 4 green = step done
```

Notes on the QEMU gate:
- Runs under pure TCG (no KVM flag), one boot per test, 60 s boot timeouts — slow. Run it **once
  per step at the end**, not on every edit.
- `cargo xtask test` runs unit + integration together (use `--skip-integration` to skip).
- Env knobs (see `tests-integration/README.md`): `ROSX_QEMU_PROFILE` (`dev`|`release`),
  `ROSX_KERNEL_ELF` (prebuilt kernel ELF), `ROSX_QEMU_KEEP_TMP`.
- **No step may be committed with a red integration run.**

**Definition of done:** all three commits independently green including the QEMU gate;
`scheduler.rs` contains no `services()` and no `crate::kernel::` adapter references; `Scheduler`
has exactly one constructor; the QEMU boot flow (shell + `ls` + keystroke echo, keyboard input,
`sleep`) behaves as before.

---

## 6. Risks & Rollback

- **R1 — Loop-semantics drift (Step 1).** The exact order drain → interrupt → timers → pick →
  switch → reconcile is load-bearing (timer futures must complete before `pick_next` so woken
  tasks are queueable in the same cycle). **Mitigation:** cut/paste the body verbatim into
  `step()`; new ordering tests pin it; QEMU gate.
- **R2 — Placeholder scheduler window (Step 2).** `kernel_services.rs:85` creates the scheduler
  before `Kernel::new` replaces it. Switching it to all-Noop ports is safe only because no
  scheduler method runs between `init()` and `replace()` — verify by grep for
  `services().scheduler` call sites (`kernel.rs:164, 205, 249, 295` — all post-replace).
- **R3 — Borrow subtleties.** `KernelCell::borrow_mut()` in the `run()` driver must not be held
  across `context_switcher.switch_to_task` (re-entrancy via interrupts). Today each call
  borrows fresh per statement; keep that shape — one `step()` call per borrow, never nest.
- **R4 — Test rewrite hides lost coverage (Step 3).** Map every removed `services()`-based test to
  its fake-based replacement before deleting; keep test count ≥ before.
- **R5 — Integration flakiness.** TCG timing + keystroke injection can be flaky under CPU
  contention. **Mitigation:** always `--test-threads=1`; on a suspected flake, rerun the single
  failing test before judging the step red.
- **Rollback:** each step is one behavior-preserving commit; `git revert <sha>` at any point.

---

## 7. Noted, Not Fixed (out of scope for this plan)

1. `reconcile_returned_task` sets `self.idle_task = Some(returned_handle)` in the idle branch
   (`scheduler.rs:188`) — redundant (already `Some`). Possible logic smell; needs its own
   discussion.
2. `start_next_task` `unwrap()`s a missing idle task (`scheduler.rs:169`) — panic path kept
   verbatim; a graceful halt/`cpu.halt()` fallback is a separate decision.
3. `algorithm: Box<dyn SchedulingAlgorithm + Send>` vs. `&'static dyn` for the other ports —
   inconsistent injection style; converting it is a follow-up.
4. xtask still runs `--test-threads=1` globally (`xtask/src/main.rs:85`); once other modules get
   the same hermetic treatment, parallel tests could be revisited.

---

## 8. Resolved Decisions

- **Q1 — scratch buffer for `step`.** **Resolved:** private buffer. `Scheduler` gains a private
  `interrupt_drain_buf: Vec<HardwareInterrupt>` field; signature is `step(&mut self)`.
- **Q2 — `SchedulerPorts` location.** **Resolved:** `scheduler.rs`, next to `Scheduler` and the
  Noop adapters.
- **Q3 — placeholder in `kernel_services.rs:85`.** **Resolved:** keep a Noop-ports placeholder
  (`Scheduler::new(FifoStrategy::new(), SchedulerPorts::noop())`), smallest diff; verified safe
  per R2.
- **Integration gate.** **Resolved:** all `tests-integration` tests must pass by the end of every
  step (see §5).

---

## 9. Out of Scope

- Behavior changes of any kind (including §7 items).
- Strategy (`SchedulingAlgorithm`) trait changes; task manager / future registry / IPC internals.
- Syscall ABI, `usrlib`, arch crates (they keep compiling unchanged).
- Moving port trait definitions to the `system` crate (possible future plan, mirroring
  `plans/ipc-hexagonal-refactor.md`).

---

## 10. How an agent picks this up (checklist)

1. Read this file. Re-verify the As-Is snapshot in §2 against the current tree (line numbers drift).
2. `git status` + `git log --oneline -15` to see where you are.
3. Run `cargo test -p kernel` to confirm the baseline is green **before** touching anything.
4. Execute steps in order; for each: make the change → `cargo test -p kernel` →
   `cargo build -p rosx` + `cargo build -p rosx-i686` → `cargo test -p tests-integration --
   --test-threads=1` → propose a commit message → commit. **One concept per commit; stop and
   confirm between steps.**
5. Keep the §3 invariants + §6 risks in view; when in doubt, the QEMU gate is the arbiter.
6. Finish by marking this plan `Status: Done` and recording any deferred items (§7).
