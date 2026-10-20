# Plan: Threading Demo App — "What Already Works, What Breaks"

> **Status:** PROPOSED — needs architect review
> **Audience:** architect agent → coder agent
> **Depends on:** nothing (pure user-space + build plumbing)
> **Blocks:** plans/threading-task-groups-option-b.md (its acceptance tests are this demo's scenarios)

---

## 1. Purpose

A throwaway-but-upstreamable demo app that empirically establishes, in QEMU, exactly which
threading capabilities RosX already provides and which are hazards waiting to be fixed.

The demo is the evidence base for the Option B kernel work (task groups, image lifetime,
blocking synchronization). It must NOT change kernel code. Every observation is recorded
(transcript excerpt in the commit message body) so the architect of Plan B can design against
facts, not theory.

## 2. Verified facts this demo rests on (all confirmed in current source)

| # | Fact | Location |
|---|------|----------|
| F1 | `Syscall::exec(fn as usize)` spawns a new schedulable task whose entry is a plain user function pointer | `usrlib/src/syscall.rs:16`, `kernel/src/syscall.rs:27-33`, `kernel/src/task.rs:253-261` |
| F2 | `Exec` returns a task-completion future handle; `terminate_and_yield` notifies it ⇒ join already exists | `kernel/src/kernel.rs:181-201,262-277` |
| F3 | All tasks run Ring 0 in one flat address space; NX disabled; `Alloc`/`Dealloc` are the kernel's global allocator ⇒ shared memory between tasks is automatic | `arch/x86_64/src/syscall.S:58`, `arch/x86_64/src/cpu.rs:23-31`, `kernel/src/syscall.rs:68-76` |
| F4 | Each task gets its own 16 KiB kernel stack with full GPR save/restore; timer preemption works and resumes exactly where stopped | `kernel/src/task_stack.rs:4`, `arch/x86_64/src/context_switching.S` |
| F5 | **Hazard:** the loaded ELF `Image` is owned by the app's first task wrapper and `drop`ed when that task returns — workers `exec`ed from that image dangle if main exits first | `kernel/src/task.rs:464-478` |
| F6 | **Hazard:** IPC connections record `client_task: TaskHandle` and enforce caller equality — a second task of the same app cannot use the app's connection | `kernel/src/ipc/ipc_manager.rs:44,133,192` |
| F7 | There is no blocking synchronization syscall; spin-wait is the only user-side wait | `system/src/syscall_numbers.rs` (no futex/mutex) |
| F8 | Shell embeds app binaries with `include_bytes!` from `target/rosx-user/release/` and `spawn()` **waits** on the spawned app's completion future | `apps/shell/src/shell.rs:13-15,106-125` |
| F9 | Precedent for exec-based workers: `test_suite` runs inside the shell's own image via `Syscall::exec` | `apps/shell/src/shell.rs:176`, `apps/test_suite/src/app.rs` |

**Critical corollary of F5+F8:** the demo must be a standalone ELF app loaded via `LoadElf`
(not a library linked into the shell, like test_suite). Only a loaded ELF task owns an `Image`,
so only a loaded ELF app can reproduce the use-after-free. Shell's `spawn()` waiting on *main*
does not help the worker: main exiting early is precisely what frees the code.

## 3. Goals

- G1: Demonstrate spawn + join + shared-memory atomics across concurrent tasks (F1, F2, F3).
- G2: Demonstrate preemption interleaving and the cost of spin-waiting (F4, F7).
- G3: Demonstrate the IPC cross-task ownership failure (F6) with the returned error code.
- G4: Demonstrate the image-lifetime use-after-free (F5) — **behind an explicit opt-in argument**.
- G5: App stays in-tree afterwards as the living regression suite for Plan B.

## 4. Non-Goals

- No kernel changes, no new syscalls, no usrlib changes.
- No SMP, no TLS, no `spawn` with arguments (that is Plan B).
- No integration-test registration yet (the scenarios currently include one that crashes by design).
- Not adding the app to ROM; shell embeds it directly like all other apps.

## 5. Design

One new workspace-member binary crate `apps/thread_demo`, launched from the shell as:

```
threads safe      -> scenario S1 (and S2, S3)
threads hazard    -> scenario S4 (explicit opt-in; crashes)
```

Behavior selection via `usrlib::args()` (the shell passes argv through `spawn` → `LoadElf`
argument blob; see `hello_elf` printing `usrlib::argc()`).

### Scenarios

**S1 — Shared counter, spawn + join.**
`static COUNTER: AtomicUsize`. Main `exec`s worker A and worker B (each does
`COUNTER.fetch_add(1, SeqCst)` 1000 times with a small `Yield` cadence), joins both futures,
prints `S1 FINAL=<n>`. Expected: `FINAL=2000`, and interleaved progress prints prove both ran.
Establishes: threads-as-tasks work today for state sharing.

**S2 — Spin-wait cost.**
Worker spins on an `AtomicBool` printed once per 10k spins (`spin ticks: N` every so often);
main `sleep(500)` then `sleeps the kernel quantum a few times on purpose (e.g. 4 x sleep(1))`,
sets the flag, joins, prints `S2 SPIN_TICKS=<n>`. Expected: worker consumes whole preemption
quanta while spinning (ticks counted across sleeps) — the quantitative argument for a park/wake
primitive (Plan B phase B4).

**S3 — IPC ownership refusal.**
Main connects to service `"RANDOM"` (server spawned separately from shell, or skipped-with-SKIP
line if connect fails); stores the `UserConnectionHandle` in a static; `exec`s a worker that
calls `Syscall::ipc_send` on that connection and prints the exact error code returned; joins;
prints `S3 WORKER_SEND=<code>`. Main then sends successfully on the same connection and prints
`S3 MAIN_SEND=ok`. Expected: worker gets the documented ownership error while main succeeds —
demonstrates F6 precisely.

**S4 — Image lifetime hazard (opt-in `hazard` argument only).**
`exec` a worker that loops 20 times printing `S4 WORKER_ALIVE <i>` with `sleep(200)` each
(≈4 s lifetime). Main prints `S4 MAIN_EXITING` and returns **immediately** (no join).
The app's `elf_task_wrapper` then drops the `Image` holding the worker's code and statics.
Expected outcomes (any of them are valid observations — record exactly what happens):
garbage printout, silent worker death, or a fault that kills the system. The freed heap block
may be reused by unrelated allocations (e.g. print formatting), so behavior is timing-dependent.

### File-by-file changes

1. **`apps/thread_demo/Cargo.toml`** — new package, `publish = false`, deps `system`, `usrlib`;
   mirror `apps/test_suite/Cargo.toml`. Name: `thread_demo`, lib-free pure `[[bin]]`.
2. **`apps/thread_demo/src/main.rs`** — `#![no_std]`, `extern crate alloc;`, scenarios S1-S4,
   dispatch on `usrlib::args()`. Worker functions are plain `fn()` items (F1). No comments —
   scenario names and prints carry the documentation (AGENTS.md CRITICAL rule).
3. **`Cargo.toml` (workspace root)** — add `"apps/thread_demo"` to `members`. Do NOT add to
   `default-members` (bare-metal bin, host-uncompilable, same as hello_elf et al.).
4. **`xtask/src/main.rs`** — add `"thread_demo"` to `USER_APPS` (const array length +1).
5. **`apps/shell/src/shell.rs`** — add
   `#[cfg(target_arch = "x86_64")] static THREAD_DEMO_ELF: &[u8] = include_bytes!("../../../target/rosx-user/release/thread_demo");`
   plus a `threads` command fn calling `spawn(THREAD_DEMO_ELF, cmd)`, registered via
   `commands.insert` behind `#[cfg(target_arch = "x86_64")]` (precedent: `ping`, `vfs`,
   `shell.rs:56-58`).

## 6. Steps (each: implement → verify → commit, in order)

- **D1.** New crate `thread_demo` with S1 only; wire workspace + xtask + shell.
  Verify: `cargo xtask build` succeeds; `cargo run -p rosx`; in QEMU shell: `threads safe`
  prints interleaved progress and `S1 FINAL=2000`.
  Commit: `Add thread_demo app demonstrating exec spawn and join with shared atomics`
- **D2.** Add S2 spin scenario. Verify as D1; record `S2 SPIN_TICKS` transcript in commit body.
  Commit: `Add spin-wait cost scenario to thread_demo`
- **D3.** Add S3 IPC ownership scenario. Verify with `random_gen_server` running (shell `random`
  command context). Record the exact error code in commit body.
  Commit: `Add IPC cross-task ownership scenario to thread_demo`
- **D4.** Add S4 hazard scenario behind the `hazard` argument. Verify `threads safe` still passes;
  run `threads hazard` once, capture transcript into commit body. Default with no args runs S1
  (so nobody crashes by accident).
  Commit: `Add ELF image lifetime hazard scenario to thread_demo behind argument`

**Build-order note for the coder:** `apps/shell` compiles on the host via `include_bytes!` of
`target/rosx-user/release/thread_demo`, so `cargo xtask apps` (or `build`) must run before a
plain `cargo test` that compiles shell — pre-existing constraint shared with hello_elf.

## 7. Verification matrix

| Check | Command |
|---|---|
| Full build incl. new ELF | `cargo xtask build` |
| Existing unit tests unaffected | `cargo xtask test` |
| Manual QEMU (all scenarios + transcript capture) | `cargo run -p rosx` → `threads safe`, `threads hazard` |

## 8. Open questions for the architect

1. Should S2's spin cadence use `Yield` between spin checks (measures only own quanta) or pure
   spin (measures preemption behavior)? Proposed: pure spin, since preemption is the mechanism.
2. S3 requires the RANDOM server; acceptable to print `S3 SKIP` if connect fails, or should the
   demo bind its own trivial service instead? Proposed: self-bound service keeps the scenario
   hermetic (uses `IpcBind` + `Accept` in main, worker connects — both halves under demo control).
3. Is `threads` the right command name, or `threads safe|hazard` collides with future
   `test_suite` growth? Proposed: keep `threads`; test_suite stays the in-shell suite.

## 9. Regression guard checklist (AGENTS.md)

- [ ] No kernel/ touched
- [ ] No comments added anywhere (assembly untouched)
- [ ] Demo is a workspace member but not a default-member
- [ ] No new third-party dependencies
- [ ] Hazard scenario unreachable without explicit argument
