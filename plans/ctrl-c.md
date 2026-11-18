# Ctrl+C — Interrupt the Foreground App

> **Status:** planned, not implemented.
> **Scope:** v1 kills exactly one thing — the shell's current foreground program — via Ctrl+C.
> **Decisions taken:** kill authority stays with the shell (waitset); decoder maps Ctrl+C only;
> Kill is addressed by the child's completion-future handle and stamps a new
> `EXIT_REASON_INTERRUPTED`; victim cleanup is MVP-grade with a named follow-up.

## 1. Problem

The shell blocks in `WaitFuture` on the foreground child's completion future
(`apps/shell/src/launcher.rs`), so it is deaf to the keyboard while a program runs.
There is no way to stop a runaway program: termination is strictly self-inflicted
(`Kernel::terminate_with_status` only acts on `current_task`), no signal concept exists,
and `ps2d` decodes Ctrl+C as a plain `'c'` (`apps/ps2d/src/decoder.rs` tracks Shift only;
`Key::LeftControl` maps to `None`).

## 2. Mechanisms we build on (all already proven in-tree)

| Mechanism | Where proven |
|---|---|
| Completion future carries exit status, stamping wakes the parked parent | `kernel/src/future/mod.rs` `notify_with_status`, exit_status suite |
| Unsolicited server→client push wakes a parked `IpcReceiveMessage` future | ne2kd `Watch` / `PushFrame` |
| Waitset over {completion future, message future}, event names the child, child stays consumable | `kernel/src/future/waitset.rs`, `waitset_or` suite |
| `wake_tasks` refuses to resurrect Terminated tasks | `kernel/src/scheduler/scheduler.rs` |
| Death reclaims port grants and the loaded-image memory group | `terminate_with_status`, `platform::portio::revoke` |
| QEMU monitor keystrokes drive the whole input loop in tests | `tests-integration/src/sendkey.rs` |

## 3. Design

Three user-space seams and one kernel primitive:

```
 IRQ1 → ps2d            inputd                    shell                     kernel
 ─────  ─────            ──────                    ─────                     ──────
 decoder emits '\x03' →  '\x03' with watcher  →    waitset {child C,    →    Kill(C):
 registered: send TAG_   interrupt recv W}          stamp completion
 CONTROL instead of      to the watcher only;        event W → shell         Interrupted,
 delivering to readers    without watcher: acts      calls Kill(C)           revoke, reclaim,
 or buffering (ISIG)                                 → C stamps → reap       remove from queue
```

### 3.1 ps2d — control-character decoding

`Ps2Decoder` gains a `controlled` flag beside `shifted`: scancode `0x1D` make clears/sets
it, `0x9D` break clears it (Right Ctrl needs the unparsed `E0` prefix — v1 is Left Ctrl only).
`char_for` gains the one rule: Ctrl held + `Key::C` press → `Some('\x03')`. Every other
Ctrl combination keeps its current plain-character output. Ctrl make/break frames emit nothing.

### 3.2 inputd — interrupt routing

New wire opcodes in `protocol/input` (`TAG_WATCH = 6`, `TAG_UNWATCH = 7`, `TAG_CONTROL = 8`).
The mux holds one watcher slot (a connection; last `WATCH` wins; cleared on `UNWATCH` or on
send failure, mirroring the existing reader-forget discipline). On `handle_push`:

- char `'\x03'`, watcher registered → send a `TAG_CONTROL` notice frame to the watcher;
  do **not** deliver to a parked reader and do **not** buffer — the foreground app never
  sees the interrupt character, matching Unix ISIG semantics.
- char `'\x03'`, no watcher → ordinary character (the shell prompt handles it).
- anything else → unchanged mux behavior.

Policy stays in inputd, scancode semantics stay in ps2d, the kernel keeps delivering
opaque frames — the input doctrine is untouched.

### 3.3 kernel — `Kill` syscall (number 42), addressed by completion future

`Kill(completion_future_handle)` makes the held token the kill capability — symmetric to
`WaitFuture`, and consistent with the exit-status doctrine that *the parent is whoever
holds the completion-future handle*.

Resolution: `TaskCompletionFuture` grows an `owner: TaskHandle` field, stamped at
`schedule_with_task_handle` pairing time. Generation-checked handles make stale, consumed,
or recycled targets fail honestly instead of killing a stranger.

Teardown order mirrors `terminate_with_status`, extended for a non-current target
(the killer is `current_task` on a uniprocessor, so the target is Ready or Blocked):

1. resolve owner via the future; slot gone or `Terminated` → `TaskKillTargetTerminated`;
   owner is `current_task` → `TaskKillTargetRunning`.
2. stamp + `notify_with_status(owner's completion, EXIT_REASON_INTERRUPTED-encoded status)`
   — status first, in the same registry section that wakes waiters (existing invariant),
   which also cascades into any waitset holding C (so a waitset-waiting parent wakes too).
3. `set_state(Terminated)`.
4. `platform::portio::revoke(target)`.
5. group reclaim guarded by `group_has_live_members` (Spawn siblings survive, as on self-exit).
6. scheduler bookkeeping the self-exit path never needed: `algorithm.on_task_terminate`
   (run-queue removal; must be idempotent for a Blocked target) and a new
   `FutureRegistry::discard_waiter(task)` scanning `waiters` (bounded by `FUTURE_SLOTS`,
   rare syscall) so a blocked target never lingers in a waiter list.
7. `task_manager.remove_task(target)` — the victim's stack frees via `Task::Drop`
   (Stack-owner reclaim rule respected: the bulk sweep never touches stacks).

IrqGuard discipline follows `terminate_with_status`: sequential section borrows, never nested,
tripwire silent.

New error codes open the task band: `TaskKillTargetTerminated = 0x0801`,
`TaskKillTargetRunning = 0x0802`.

### 3.4 exit status

`system::exit` gains `EXIT_REASON_INTERRUPTED = 2` (code bits 0). `usrlib` decodes it as
`ExitStatus::Interrupted`; the shell reports `interrupted` beside the existing
`finished` / `exited with code N` / `panicked`. Because the stamp happens inside `Kill`,
the parent's existing reap path needs no special case at all.

### 3.5 shell — watcher lifecycle and the split wait

Around a foreground ELF launch (`launcher.rs`):

1. open a dedicated INPUTD connection for control traffic (the per-process `read_char`
   connection stays reserved for `transact`, so replies never cross wires), send `WATCH`.
2. arm `IpcReceiveMessage(watcher_conn)` as W; `WaitsetCreate`; attach C (the child's
   completion future) and W with distinct tags; `WaitsetWait` loop:
   - event for C → reap via the consuming `wait_for_exit_status(C)`;
   - event for W → consume W, decode `TAG_CONTROL`, call `Kill(C)` (an already-terminated
     child is an idle no-op — covers the exit/Ctrl+C race), then keep waiting for C.
3. `WaitsetDrop` + `UNWATCH` when the child is reaped, then report as today.

At the prompt (no watcher registered) a delivered `'\x03'` reaches the prompt loop:
print `^C`, discard the partial line buffer, reprint the prompt — Unix-familiar.

Background jobs (`&`) are untouched: Ctrl+C is a foreground-only event, like Unix.
Built-ins run inside the shell task itself — nothing to kill; during a built-in the watcher
is not registered, so a stray `^C` is handled by the prompt loop at the next boundary.
A `TAG_CONTROL` racing the final reap is purged by the detach-pending-on-drop semantics of
waitsets; the retained-pending-future discipline (ne2kd lesson) applies to W across
waitset iterations.

## 4. Non-goals and known residue

- **Victim's orphaned kernel objects.** A task killed mid-IPC leaves its unconsumed
  receive future (and inputd's parked read entry) behind — the same leak class self-exit
  already tolerates (no `FutureDrop`; IPC bindings survive death today). Ports and memory
  groups are reclaimed; per-future owner tracking and full victim GC are a named follow-up
  plan, and repeated Ctrl+C on IPC-parked apps can eventually consume `FUTURE_SLOTS`.
- Not signals: no SIGTERM family, no delivery to process groups, no handler registration,
  no Ctrl+Z / Ctrl+D. Other Ctrl combos unchanged. Right Ctrl unsupported (E0 prefix).
- Kernel-native signal delivery stays rejected: input semantics live in user space.
- `ps`/kstatsd need no change (a killed group simply vanishes on the next snapshot).

## 5. Increments (each verified before the next)

1. **ps2d decoder**: `controlled` state, Ctrl+C → `'\x03'`, release semantics.
   Verify: `cargo test -p ps2d` (new decoder unit tests).
2. **input-protocol wire**: `TAG_WATCH` / `TAG_UNWATCH` / `TAG_CONTROL` + codec.
   Verify: host unit tests, `cargo xtask test`.
3. **inputd mux**: watcher slot, interrupt routing, no-watcher passthrough, send-failure
   forget. Verify: `cargo test -p inputd` mux unit tests.
4. **system treaty**: `SyscallNum::Kill = 42`, task-band error codes,
   `EXIT_REASON_INTERRUPTED` + encode/decode tests. Verify: `cargo xtask test`.
5. **kernel**: owner stamp on `TaskCompletionFuture`, `Kernel::kill_task`, `Kill` dispatch,
   `FutureRegistry::discard_waiter`, idempotent `on_task_terminate` for unqueued handles.
   Verify: `cargo test -p kernel -- --test-threads=1` (kill Ready / kill Blocked /
   kill-dead / kill-self / group-with-siblings / port-revoke unit tests).
6. **usrlib**: `Syscall::kill(handle)`, `ExitStatus::Interrupted`.
   Verify: user builds on both arches.
7. **shell**: watcher lifecycle, split waitset wait, `Kill` on interrupt, `^C` prompt
   handling, `interrupted` report. Verify: `cargo xtask build` + `cargo xtask check-isa x86_32`,
   manual `cargo xtask run` on both arches (`pi` is a perfect live victim: prints `Pi: ` then
   spins in pure computation).
8. **tests-integration**: `ctrl-c` token in `src/sendkey.rs` (verify the guest sees
   `0x1D`-make + `0x2E`-make — human-monitor token or a hold-down sequence); new
   `apps/spinner` test carrier (prints `spinner ready`, loops `sleep` forever — the
   `exiter` precedent for test-carrier apps, 8.3-safe, joins the full-apps FAT master
   automatically); new `tests-integration/tests/ctrl_c.rs`:
   - kill foreground: `spinner` → ready → `^C` → prompt returns, report `interrupted`, shell alive;
   - `^C` at a bare prompt → `^C` echoed, fresh prompt, shell alive;
   - Ctrl+C kills the foreground victim while the boot-plan services (dials unmolested) and
     the next prompt keep working; a background job (`spinner &`) keeps running through an
     unrelated interrupt and reaps only at its own exit.
   Verify: `ROSX_KERNEL_ELF`-pinned runs, both arches, `--test-threads=1`.
9. **docs**: AGENTS.md updates (input architecture gains the watcher/control routing,
   exit-status channel gains `Interrupted`, syscall roster gains 42, VFS-of-the-console
   paragraph on the shared console) and `plans/ctrl-c-impl.md` evidence log.

## 6. Risk notes

- **Scheduler teardown of a non-current task is new ground** — `remove_task` today only
  ever runs on a just-returned self-terminated task. Increment 5 starts with a unit-test
  spike proving Ready and Blocked removal paths before the syscall exists.
- **QEMU `sendkey ctrl-c` fidelity** is unverified; pin the scancode truth in ps2d unit
  tests first so an integration failure means the transport, not the decoder.
- **Watch-vs-buffer races** (Ctrl+C landing between launch and WATCH registration) degrade
  to the character landing in the inputd buffer — visible, bounded, harmless.
