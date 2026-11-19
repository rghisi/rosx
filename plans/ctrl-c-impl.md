# Ctrl+C — implementation evidence log

Branch: `ctrlc-interrupt`. Parent document: `plans/ctrl-c.md`.
Running log of every milestone gate: what was run, what it printed, what it
proved. Host machine: QEMU 10.2.1, rustc 1.101.0-nightly `0abfedbc7` 2026-10-02.

## Decisions taken (per the plan's recommendations)

- **D1:** the shell owns the kill: it registers an INPUTD watcher only around
  foreground launches and waits a waitset over {child completion future,
  control receive}. Kill authority stays with the parent holding the handle.
- **D2:** the decoder maps Ctrl+C only; every other Ctrl combination keeps its
  plain-character output.
- **D3:** `Kill` is addressed by the child's completion-future handle and
  stamps `EXIT_REASON_INTERRUPTED = 2`.
- **D4:** MVP cleanup only (port grants revoked, memory group reclaimed if the
  victim was the last live member). Orphaned-future and IPC-connection GC is a
  named follow-up, see "Deferred cleanup" below.

## Refinements taken during implementation

- **`TAG_UNWATCHED = 9`** was added to the input wire (the plan reserved none).
  A pending `IpcMessageFuture` can only be released by a message completing it
  — there is no FutureDrop syscall and `mailbox_manager.remove` does not
  complete pending waiters — so `UNWATCH` always answers one `UNWATCHED` frame
  that the shell drains after the waitset loop before disconnecting. Without
  the ack every foreground launch would leak one future slot.
- `TaskCompletionFuture::new()` became `owned_by(owner: TaskHandle)`: the
  completion future carries its task from pairing time, which is what gives
  `Kill` its resolve-handle-to-task path (`FutureRegistry::completion_owner`).
- `Scheduler::handle_termination` became `pub(crate)` so `Kernel::kill_task`
  removes the victim from the scheduling queues on the same path a voluntary
  exit uses.

## M1 — ps2d decoder (`4ff07ab`)

- Edit: `Ps2Decoder` gained `controlled` beside `shifted` (`0x1D` make sets,
  `0x9D` break clears); `char_for` gained `Key::C` + control → `'\x03'`.
- Gate: `cargo test -p ps2d` — 20/20 green. New tests pin: Ctrl+C emits
  `'\x03'`; shift does not change it (either key order); Ctrl make/break emit
  nothing; release clears the state; repeated C presses under held Ctrl keep
  interrupting; Ctrl+other keys stay plain chars and survive other keys'
  release frames.

## M2 — input wire + inputd mux (`24ab12a`, `a62e0a0`)

- Edits: wire tags `WATCH = 6`, `UNWATCH = 7`, `CONTROL = 8` (+ `CONTROL_C`
  constant), later `UNWATCHED = 9` with `encode_unwatched()`; mux gained a
  watcher slot: `handle_push` routes `'\x03'` to the watcher when one is
  registered (not to readers, not to the buffer), keeps it an ordinary char
  without one; `handle_unwatch` evicts and acks; `forget` evicts a dead
  watcher; the daemon dispatch serves both new tags.
- Gate: `cargo test -p input-protocol -p inputd` — 8 + 17 green. New tests
  pin: watcher notified and reader/buffer see nothing; no-watcher path
  byte-identical to the old mux; last WATCH wins; foreign UNWATCH is a no-op
  on the slot (still acked); UNWATCHED returns `'\x03'` to the ordinary path;
  forget-evicted watcher falls back to ordinary; other chars keep flowing to
  readers while a watcher is registered.

## M3 — system treaty (`6482255`)

- Edits: `SyscallNum::Kill = 42`, `ErrorCode::TaskKillTargetTerminated =
  0x0801`, `TaskKillTargetRunning = 0x0802`, `EXIT_REASON_INTERRUPTED = 2`
  with `interrupted_status()`. The reserved-reason decode test moved to reason
  3 now that 2 is taken.
- Gate: `cargo test -p system` — 101 green.

## M4 — kernel kill path (`3a10aae`)

- Edits: `TaskCompletionFuture::owned_by` (owner stamped at
  `schedule_with_task_handle` pairing), `FutureRegistry::completion_owner` +
  `discard_waiter`, `Kernel::kill_task`, `KillError`, the `Kill` dispatch arm,
  `Scheduler::handle_termination` visibility.
- Ordering invariant (same as `terminate_with_status`): resolve owner → refuse
  `current_task` and `Running`/`Terminated` targets → **stamp `Interrupted(0)`
  and notify the completion future first** → state Terminated →
  `discard_waiter` (a blocked victim can never be resurrected — `wake_tasks`
  refuses Terminated) → `portio::revoke` → group reclaim when the victim is
  the last live member → `handle_termination`.
- Gate: `cargo test -p kernel -- --test-threads=1` — 433 green. Eight kill
  tests pin: Ready victim stamps Interrupted and reaches Terminated; a parent
  parked on the completion future wakes; a Blocked victim stays dead after its
  gate fires; second kill reports TargetTerminated; killing the current task
  is refused as TargetRunning; foreign/absent future handles are refused;
  revoked port range is acquirable by the next task; group memory survives
  the first of two members and is reclaimed by the second's death. Three
  future tests pin `completion_owner` (rejects non-completion and absent
  handles) and `discard_waiter` (evicts from every list, later notify does not
  re-add).

## M5 — usrlib (`fd69665`)

- Edits: `Syscall::kill(task_completion)`, `TaskKillError` with the
  `From<ErrorCode>`/Display pair wired into the all-codes table test,
  `ExitStatus::Interrupted` decoding.
- Gate: `cargo test -p usrlib` — 46 green.

## M6 — shell (`c70456a`)

- Edits: `launcher.rs` gained `ControlWatcher::open/close` (connect INPUTD,
  WATCH; UNWATCH + disconnect) and `wait_for_foreground` (waitset over
  {child, control receive}); a `CONTROL` frame echoes `^C`, echoes nothing
  else, and calls `Syscall::kill(child)`; after the loop the UNWATCH ack
  drains the still-pending receive so the future slot is released. Foreground
  `Interrupted` is reported silently — the `^C` echo IS the report.
  `shell.rs`: bare-prompt `'\x03'` arm echoes `^C`, clears the line buffer and
  re-prompts; `report_finished` gained the honest `Interrupted` arm.
- Gate: `cargo xtask apps` green (all 41 user ELFs); host-side inputd +
  wire tests green.

## M7 — test carrier + integration suite (`2794937`)

- `spinner` app: prints `spinner ready`, sleeps 100 ms per tick, announces
  `spinner tick N` every 10 ticks, never exits — the deterministic victim.
  Added to `USER_APPS` (now 41); stays OUT of `BOOTSTRAP_APPS` (the interrupt
  tests run from the disk shell).
- `sendkey.rs`: `'\x03' => "ctrl-c"`. Verified against the real QEMU 10.2.1
  human monitor: `sendkey bogus-key-name` answers `invalid parameter: bogus`
  while `sendkey ctrl-c` returns clean — the token exists.
- `tests-integration/tests/ctrl_c.rs`, four scenarios:
  - kill of a running foreground spinner: `^C` appears, prompt returns, the
    killed app's own output stops, and the shell still runs `exiter 7`
    afterwards;
  - `^C` at a bare prompt: `^C` echo, prompt usable;
  - `^C` after a partial line `exiter pan`: line discarded, next command
    unaffected, no panic executed;
  - background jobs spared: `spinner &` then `^C` after an unrelated `pi` run
    — the background spinner keeps ticking, never reported finished or
    interrupted.
  Note `send_text`/`send_line` ack keys by echo, which `'\x03'` never
  produces, so the suite sends it with `session.send_key('\x03')` directly.

## Final gates (both arches)

- x86_64: `ROSX_KERNEL_ELF=… cargo test -p tests-integration --test ctrl_c`
  — 4/4 green (62.8 s).
- x86_64 regressions: `boot_and_shell` 4/4, `exit_status` 5/5,
  `shell_background` 5/5, `shell_command_arguments` 1/1.
- x86_32: `cargo xtask build x86_32` green (kernel, apps, ROM, FAT boot
  disk); `cargo xtask check-isa x86_32` — "kernel and 41 user ELFs are
  486-clean"; `ROSX_ARCH=x86_32` ctrl_c 4/4, exit_status 5/5,
  shell_background 5/5 (all `--test-threads=1`).
- `cargo xtask test --skip-integration` — 61 result tables, zero failures.

## Deferred cleanup (follow-up plan territory)

The kill reclaims exactly what the voluntary-exit path reclaims (ports +
memory group). A killed task additionally leaves behind:

1. **Orphaned futures** it registered (time futures, pending receive futures
   attached or not) stay in the `FutureRegistry` forever — nothing GCs futures
   by owner task.
2. **IPC connections** it held stay open in the daemon's view; services keep
   any state keyed to them (ne2kd watchers, parked pushed data objects). The
   killed `spinner` holds none today, which is why the MVP is safe.
3. The shell leaks one future slot on one fault path only: if the UNWATCH
   send itself fails while the control receive is still pending, that receive
   cannot be drained (no FutureDrop).

A `Kill`-aware resource sweep — futures GC'd by owner, connection teardown
notices to services — is the natural next plan, shared with the `ipc-volume-
wedge` hygiene work.

**Resolved by `plans/kill-resource-sweep.md`** (evidence log
`plans/kill-resource-sweep-impl.md`): one reap runs on every death path, futures
are GC'd by owner, connections close and data refunds follow the dead task, and
items 1–3 above are closed — services learn of a dead peer through failed sends,
not a notice opcode.
