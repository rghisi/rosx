# Plan: Option B — Task Groups, Image Lifetime, and Blocking Sync for App Threads

> **Status:** PROPOSED — needs architect review
> **Audience:** architect agent → coder agent
> **Depends on:** plans/threading-demo-app.md (its scenarios S1–S4 are this plan's acceptance tests)
> **Theme:** make "an app with multiple threads of control" a real kernel concept without
> pretending we have address-space isolation we don't have.

---

## 1. Purpose

Today RosX has tasks but no apps: every `Task` is an orphan identity. Three concrete defects
follow (all verified, see facts table), and each maps to one kernel phase:

| Defect | Kernel fix | Phase |
|---|---|---|
| No app identity — IPC ownership is per task, so a second thread of an app cannot use its connections | `TaskGroup` identity in the kernel | B1, B3 |
| ELF `Image` is dropped when the app's first task returns, freeing code that still-running workers execute | image ownership by reference count, inherited by exec'd tasks | B2 |
| No blocking synchronization — threads spin or misuse Sleep | park/wake (futex-style) futures + usrlib mutex/semaphore | B4 |

Thread-argument plumbing and identity for user code is B5 (optional, last).

## 2. Verified facts (current source)

- F1 `Task::new(name, entry_point, entry_param)` — no group field; call sites:
  `kernel/src/kernel.rs:119,163` (via factories), `kernel/src/task.rs:156,163,175,245,250`,
  syscall paths `kernel/src/syscall.rs:27-33,80-97`.
- F2 `Exec` spawns `new_entrypoint_task(entrypoint)` — child has no link to the calling task
  besides none at all; returns completion future (`kernel/src/kernel.rs:181-201`).
- F3 `elf_task_wrapper` (`kernel/src/task.rs:464-478`) loads the image lazily on the new task's
  own stack, owns it as a local, and drops it at task end ⇒ F5-hazard.
- F4 IPC: `client_task: TaskHandle` stored per connection and checked for equality in
  `kernel/src/ipc/ipc_manager.rs:44,133,192`; data-token read/write/release are likewise
  caller-checked (`kernel/src/syscall.rs:133-161`).
- F5 `TaskManager` is a `GenerationalArena<SharedTask, 256>`
  (`kernel/src/task_manager.rs:7-9`); removal happens in `Scheduler::handle_termination`
  (`kernel/src/scheduler/scheduler.rs:167-173`).
- F6 Futures infrastructure is notification-driven: registry with
  `register_waiter`/`notify` → `wake_tasks` (`kernel/src/future.rs:98-162`) — a park/wake
  primitive can reuse it entirely.
- F7 Treaty is numbers + layouts only: `system/src/syscall_numbers.rs` ends at `RomRead = 27`;
  error codes in `system/src/error.rs`.
- F8 All kernel shared state lives behind `KernelCell`/`RefCell` in
  `kernel/src/kernel_services.rs` + `kernel/src/kernel_cell.rs` — a deliberate single-core
  assumption. Keep it that way; new state must live in the same place so a future SMP plan has
  exactly one place to introduce locking.

## 3. Goals and Non-Goals

**Goals**
- G1 One app's threads share identity: IPC connections, data tokens, and the loaded image work
  from any of the app's tasks.
- G2 An app's image lives until its last task ends (no use-after-free; demo S4 goes green).
- G3 Race-free blocking synchronization: user-space Mutex/Semaphore/Condvar built on a kernel
  park/wake primitive, no spinning.
- G4 All new kernel subsystems unit-tested in place (AGENTS.md CRITICAL), kernel stays portable
  (`no_std`, only `system` + `lazy_static`), HAL untouched — none of B1–B4 is arch-specific.

**Non-Goals**
- No address-space isolation, no Ring 3, no guard pages (separate future arc).
- No SMP. This plan must not *widen* the single-core assumption: no new `static`s outside
  `kernel_services`, no new `KernelCell` users, nothing that a later per-CPU refactor would
  have to chase across modules.
- No POSIX `pthread` emulation, no thread priorities, no thread-local storage in B1–B4
  (B5 sketches TLS as the optional closer).
- The kernel never interprets message payloads — unchanged invariant.

## 4. Architecture placement (principles check, AGENTS.md)

- *Domain knowledge owned by the allocating subsystem:* `TaskGroup` identity/layout/validity is
  owned by the task subsystem in `kernel/` — it describes **how the kernel works**, so it does
  NOT go into `system` (treaty holds contract, not mechanism).
- *Kernel is the single authority on invariants:* group membership, image refcounting, and the
  futex compare-and-block decision happen kernel-side, once; user space carries opaque tokens.
- *Translation at boundaries:* only two new syscall numbers and (if needed) error codes enter
  `system/`. No new wire layouts — futex args are plain scalars and addresses (legal because
  the address space is flat; that fact is kernel mechanism, not treaty).
- *No kernel internals in user APIs:* users never see `TaskGroupHandle`. The user-visible
  surface is: existing syscalls start working across app threads, plus `FutexWait`/`FutexWake`.

## 5. Phase B1 — TaskGroup identity (kernel-internal, no ABI change)

**Design sketch.**
- New `TaskGroupHandle` newtype in `kernel/src/handle.rs` (joins the existing five).
- New `TaskGroup { members: usize }` (refcount of live tasks) stored in a
  `GenerationalArena` inside `kernel_services` (`services().task_groups`).
- `Task` gains `group: TaskGroupHandle`. Make it explicit in `Task::new` — every task is created
  in a context that knows its group; no default-group magic.
- Bootstrap: `Kernel::new` creates one boot group (main thread, idle task; its refcount is
  pinned by construction — boot tasks live forever).
- `SyscallNum::LoadElf` → creates a **fresh** group (loading an app = starting an app).
- `SyscallNum::Exec` → child inherits the calling task's group (the syscall can read
  `kernel().execution_state.current_task()` already; extend `TaskManager` with
  `get_group(handle)`).
- `remove_task` path (`Scheduler::handle_termination` → `TaskManager::remove_task`) decrements;
  at zero the group arena slot is released.

**Steps** (each verified + committed; verification always: `cargo test -p kernel --
--test-threads=1`, `cargo xtask test`, QEMU smoke via shell `tests`):
- B1.1 `TaskGroupHandle` newtype + registry in `kernel_services` + unit tests
  (insert/refcount/release/generational validity).
- B1.2 `Task.group` field, `Task::new` signature update across all call sites; boot group in
  `Kernel::new`/`setup`. Unit tests updated + refcount on add/remove.
- B1.3 `Exec` inherits caller group; `LoadElf` creates new group; unit tests asserting
  inheritance (`TaskManager::get_group`) and fresh-group-per-ELF.
- Suggested commits: `Add TaskGroup handle and registry`, `Attach every task to a task group`,
  `Inherit task group on Exec and mint it on LoadElf`.

**Exit criterion:** kernel unit tests prove identity flows correctly; all existing behavior
identical (groups are invisible elsewhere yet).

## 6. Phase B2 — Image lifetime follows the app, not one task

**Design sketch.**
- Move `load_elf` + relocations from `elf_task_wrapper` into the `LoadElf` syscall path
  (relocation base = stable `Vec` pointer — already true; loading during the syscall also lets
  a malformed ELF return an error code immediately instead of killing a spawned task: an
  improvement consistent with "validate once, kernel-side").
- `Image` is wrapped in `alloc::sync::Arc<Image>` (single-core today; `Arc` chosen so the SMP
  plan later does not have to revisit this).
- `Task` gains `code_origin: Option<Arc<Image>>`:
  - ELF task: the Arc created by `LoadElf`.
  - Exec'd task: clone of the spawning task's `code_origin` (the exec entry point lives in the
    spawner's code by construction — that is what `Syscall::exec(fn_ptr)` means).
  - Boot/kernel tasks: `None`.
- `elf_task_wrapper` no longer owns a local `Image`; its context holds the `Arc` and dropping
  the task drops one reference. Last task holding the Arc frees the image.

**Steps**
- B2.1 Load ELF during `LoadElf` syscall; `ElfTaskContext` holds `Arc<Image>`; wrapper uses it.
  Unit tests: malformed ELF → error code at syscall (new `ErrorCode` variant; audit
  `system/src/error.rs` for free codes first); relocation base stable across context moves.
- B2.2 `Task.code_origin` + clone on `Exec`; drop on terminate is implicit via task removal —
  prove refcount semantics with unit tests (two clones, drop main, image buffer intact; drop
  last, freed — assertable via `Arc::strong_count`).
- B2.3 Run demo `thread_demo threads hazard` — scenario S4 must now print
  `S4 WORKER_ALIVE 0..19` to completion while main exited first. This is the phase's acceptance
  test; capture transcript in commit body.
- Suggested commits: `Load ELF images during the LoadElf syscall`,
  `Keep the ELF image alive as long as any task executes it`.

**Watch-out:** termination paths must not double-drop; `terminate_and_yield` already isolates the
state transition — the Arc drop rides normal task removal in the scheduler, no new unsafe.

## 7. Phase B3 — IPC ownership by group

**Design sketch.**
- Connection record: `client_task` → `client_group`; `connect` stores the caller's group.
  Send/write/read/release checks become "caller's group == connection's group" (or, for
  server-side checks like `ipc_manager.rs:192`, "caller's group == the binding owner's group").
- Bindings: record the binding owner's group; any task in that group may `accept`/`receive`.
  The wake path is unchanged — futures still wake the specific *task* that registered, which is
  correct (one thread wakes, others keep doing their work).
- **Policy decision required from architect (document before coding):** concurrent receives by
  two threads on the same connection. Options:
  (a) first waiter wins, second gets the next message (pure extension of today's semantics);
  (b) reject the second registration with a busy error.
  Proposed: (a) — matches the mailbox model and needs no new state.
- Audit task: enumerate every task-identity comparison in `kernel/src/ipc/` (manager, mailbox,
  data manager) and re-express it in group terms; the audit list goes into the commit body.

**Steps**
- B3.1 `connect`/send path ownership → group; unit tests (same group other task: allowed;
  different group: refused with the existing error code).
- B3.2 Bindings + receive/accept + data tokens → group; unit tests per family.
- B3.3 Demo scenario S3 flips: worker's send must now print `S3 WORKER_SEND=ok`.
  Commit body records the flip.
- Suggested commits: `Attribute IPC connections to task groups`,
  `Attribute IPC bindings and data tokens to task groups`.

## 8. Phase B4 — Park/wake: the blocking-synchronization primitive

**Design sketch.**
Race-free waiting needs the *compare and enqueue* to be one uninterruptible kernel step — the
kernel is exactly the right place (single-core + interrupts masked during syscall handling makes
`if *addr == expected { block }` atomic by construction; flat memory makes the read legal).

- New `Parker` future type in `kernel/src/future.rs` (or `kernel/src/futures/` if the architect
  prefers to split; keep it in the future family so `WaitFuture`/registry machinery is reused).
- New kernel-side registry `services().park_registry: BTreeMap<u64 /*key*/, Vec<(FutureHandle,
  TaskHandle)>>` — key is the user address (futex-style).
- Two new syscalls (numbers continue the treaty; next free after `RomRead = 27` are 28/29 —
  verify no reservation in `system/src/syscall_numbers.rs` before assigning):
  - `FutexWait(addr, expected)`: if `*addr != expected` return `WouldBlock` error code (new
    variant, or audit for existing); else register parker, block current task, on wake return 0.
  - `FutexWake(addr, count)` → pops up to `count` waiters, notifies via the existing
    `FutureRegistry::notify` path, returns number woken (a count, not an error — document in the
    TryFrom/match structure itself by naming).
- `usrlib/src/sync.rs` (new module, re-exported): `Mutex` (CAS acquire on `AtomicBool`/
  `AtomicI32`, `FutexWait(&flag, LOCKED)` loop), `Semaphore` (count + waiters via same
  pattern), `Condvar` (wait key = the mutex word; `notify_one/all` → `FutexWake`). Built only on
  `core::sync::atomic` + the two syscalls — the standard futex recipe, no kernel policy.
- Terminated waiters: `wake_tasks` already skips terminated tasks; park registry must tolerate
  dead `TaskHandle`s (generation-checked handles make stale entries harmless).

**Steps**
- B4.1 `Parker` future + park registry + kernel unit tests: compare-mismatch returns without
  blocking; wake-with-zero-waiters returns 0; wake count honored; stale handle ignored.
- B4.2 Syscalls wired in `kernel/src/syscall.rs` + treaty numbers + error code; `usrlib`
  wrappers with host-side unit tests only for pure helpers.
- B4.3 `usrlib/src/sync.rs` Mutex + Semaphore + Condvar; QEMU validation through a new
  `test_suite` scenario (exec'd worker + main contend a Mutex-guarded counter ×N, assert exact
  total; a Semaphore scenario with a bounded producer/consumer over a static ring).
- B4.4 Demo S2 re-run: replace spin with `Mutex`/park — `S2 SPIN_TICKS` collapses to ~0,
  transcript in commit body. This is the quantitative proof for the whole phase.
- Suggested commits: `Add park registry and futex-style wait/wake syscalls`,
  `Add user-space mutex, semaphore and condvar built on futexes`.

## 9. Phase B5 — (Optional) thread arguments and identity for user code

- `Exec` ABI extension: `Exec(entry, arg)` — user entry becomes `extern "C" fn(usize)`;
  `usrlib::thread::spawn(fn(usize), arg)` wrapper. The plumbing exists: `Task` already carries
  `entry_param` and `Cpu::initialize_stack` already accepts `param2` (`kernel/src/cpu.rs:6-12`).
- `usrlib::thread::current()` returning an opaque thread token derived from the per-group
  sequence number minted in B1 (mints in kernel, user holds opaque token only).
- TLS (later, arch work): per-task `tls_base` switched inside `swap_context` via a new HAL
  `Cpu` capability — explicitly deferred, listed here only so B1/B5 naming doesn't box it in.

No commitment taken; architect may split this into its own plan file.

## 10. Verification matrix (every phase)

| Check | Command |
|---|---|
| Kernel unit tests | `cargo test -p kernel -- --test-threads=1` |
| Full workspace unit tests | `cargo xtask test` |
| Full build (kernel + apps + image) | `cargo xtask build` |
| QEMU behavior proof (per-phase acceptance) | `cargo run -p rosx`: shell `tests`, `threads safe`, and for B2: `threads hazard` |
| IPC apps regression (B3 specifically) | shell `random`, `ping`, `vfs` flows |

## 11. Open questions for the architect

1. **Naming:** `TaskGroup` vs `ProcessGroup` vs `App`. Proposed `TaskGroup` — it says exactly
   what it is and avoids POSIX "process" baggage we cannot honor (no isolation).
2. **B2 load placement:** moving `load_elf` into the syscall runs it on the scheduler thread's
   stack with a potentially large `Vec` allocation — acceptable? (It already runs during
   `LoadElf` handling today inside the spawned task; the move only changes *which* stack.)
3. **Concurrent-receive policy** (B3): (a) first waiter wins vs (b) busy error. Proposed (a).
4. **Futex key width:** user address as `u64` key — fine for x86_64 and x86_32 (addresses
   zero-extend); confirm before the treaty number is frozen.
5. **B4 future type:** extend `future.rs` vs new `kernel/src/futures/` module split. Prefer the
   smallest diff that keeps `future/` coherent.
6. Should B1 ship *before* B2 or combined? Proposed separate: B1 alone is testable and zero-risk;
   B2's acceptance depends on B1 only for naming coherence, not mechanism.

## 12. Regression guard checklist (AGENTS.md)

- [ ] `kernel/` stays `no_std`, deps unchanged (`system`, `lazy_static` only)
- [ ] No platform-specific code enters `kernel/` (B1–B4 are arch-neutral; B5's TLS is HAL-gated)
- [ ] Treaty additions are numbers/codes only — no kernel mechanism in `system/`
- [ ] User-visible APIs expose opaque tokens only; debug aids stay kernel-side
- [ ] No comments anywhere (except assembly, untouched by B1–B4)
- [ ] Every new/changed kernel module gains `#[cfg(test)]` tests (reference:
      `kernel/src/scheduler/mlfq_strategy.rs`)
- [ ] New shared state lives in `kernel_services` behind existing cell pattern (SMP will find it there)
- [ ] One step, one commit, no prefix; confirm before next step
