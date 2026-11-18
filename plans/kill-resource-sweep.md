# Kill-Aware Resource Sweep — Reap Everything a Dying Task Owns

> **Status:** proposed.
> **Scope:** v1 reclaims every kernel resource a dying task owns — futures by
> owner, IPC connections and data objects by task, parked mailbox waiters and
> timer-queue entries — on **both** death paths (voluntary `Exit` and `Kill`).
> **Decisions taken:** the reap is one shared kernel sweep, not a kill special
> case; services learn of a dead peer through failed sends, not a new notice
> opcode; mailbox removal consumes the futures parked on it; the victim's
> completion future survives the sweep so its holder still collects the exit
> status.
> **Parent documents:** `plans/ctrl-c.md` / `plans/ctrl-c-impl.md` (the
> deferred-cleanup section this plan closes), `plans/ipc-volume-wedge.md`
> (shared resource-hygiene family).

## 1. Problem

`Kill` (PR #176) and voluntary `Exit` reclaim exactly two resource families —
port grants and the loaded-image memory group. Everything else a task owns
survives its death forever. The ctrl-c evidence log (`plans/ctrl-c-impl.md`,
"Deferred cleanup") names the residue; the same list applies to a panicking
daemon, which is the everyday death. Measured against the current code:

1. **Orphaned futures.** Only `TaskCompletionFuture` records its owning task.
   `TimeFuture`, `IpcMessageFuture` and `WaitsetFuture` are owner-less, the
   registry has no task→futures index, so nothing can find — let alone free —
   the futures a dead task registered. A killed sleeper leaves its `TimeFuture`
   in the arena *and* its handle in `TimerManager::next`; a killed client leaves
   its pending receive parked in `mailbox_manager.waiters`.
2. **Open IPC connections.** `IpcConnection` records `client_task` but nobody
   enumerates by it. A dead client's connection, client mailbox, data objects
   and `IpcDataManager::outstanding` charge stay forever — the writer budget is
   only released through `disconnect`, which a dead task never calls. Worse,
   services pushing to the dead client *succeed* into a live orphan mailbox
   (depth 32, then congestion), so the eviction arms daemons already wrote for
   `ConnectionNotFound` (ne2kd watcher fan-out, inputd `forget`) never fire.
3. **The shell's UNWATCH-fail leak.** When `wait_for_foreground` cannot send
   `UNWATCH` while its control receive is still pending, `close()` disconnects
   over a parked future; `MailboxManager::remove` drops the waiters entry
   without touching the futures, so the slot is unreachable and leaked. The
   early-bail paths (waitset create/attach failure) leak the same way even when
   the send succeeds.

## 2. Mechanisms we build on (all already proven in-tree)

| Mechanism | Where proven |
|---|---|
| Death sweeps a per-task table (`portio::revoke`) | `platform::portio.rs`, kill + exit paths |
| `wake_tasks` refuses to resurrect Terminated tasks | `scheduler.rs` |
| `discard_waiter` evicts a task from every waiter list | `future/mod.rs`, kill path |
| `consume` removes a future and detaches every waitset parent edge | `consume_inner` |
| Stamp-before-teardown keeps waiters whole | `terminate_with_status`, `kill_task` |
| Connection teardown reclaims data objects and refunds writers | `disconnect` → `gc_connection` |
| Daemons evict dead peers on `ConnectionNotFound` sends | ne2kd fan-out, inputd `forget` |
| Generation-checked handles make stale tokens fail honestly | `GenerationalArena` |

## 3. Design

One reap, called from both death paths, ordered so the shipped invariants
survive: the completion future is stamped and notified **before** any teardown,
and `set_state(Terminated)` happens before anything that could wake the victim.

```
 terminate_with_status(status)              kill_task(completion)
        │ notify completion (status)             │ resolve owner, refuse self/Running
        ▼                                        ▼
   set_state(Terminated)  ──┐        ┌── stamp Interrupted + notify completion
                            ▼        ▼
                    reap_task_resources(task)
                       │ discard_waiter(task)        (now on both paths)
                       │ close_connections_of(task)  (client side, = disconnect)
                       │ gc_task(task)               (objects it wrote on others' conns)
                       │ discard_owned(task) → handles it consumed
                       │     prune mailbox waiters + timer queue by handle
                       ▼
                    portio::revoke → group reclaim → scheduler removal
```

### 3.1 Future ownership (`kernel/src/future/`)

`FutureRegistry::register` takes an owner `TaskHandle` and records it in a
`by_owner: BTreeMap<TaskHandle, Vec<FutureHandle>>`; `consume_inner` prunes the
reverse edge, so every destruction path (WaitFuture, WaitsetDrop, sweep,
mailbox abandonment) keeps the index honest. Owners are threaded honestly at
each production site — syscall arms pass the current task, `IpcManager`
receive/accept pass the `caller` they already hold, completion pairing passes
the paired task, and the `ForNotifyingFutures::register` trait gains the
parameter. No future is registered by anyone who is not a task, so the stamp
is exact, not heuristic.

`FutureRegistry::discard_owned(task)` consumes every future the task owns
**except the completion future its holder still needs** (the completion handle
is named by the caller so the sweep can spare it): each is removed from the
arena, detached from waitset parents (existing `consume_inner` behavior), and
its handle is returned so the caller can prune the edges the registry cannot
see — mailbox waiters and timer-queue entries. Pruning those lists matters
beyond tidiness: a stale handle in `mailbox_manager.waiters` would swallow a
delivered message, and a stale timer entry that outlives 2^15 slot wraps could
complete someone else's future.

### 3.2 Connections and data by task (`kernel/src/ipc/`)

`IpcManager::close_connections_of(task)` enumerates the connections whose
`client_task` is the dying task and tears each down exactly as `IpcDisconnect`
does — client mailbox removed, connection removed, `gc_connection` reclaims
objects and refunds each writer. `IpcDataManager::gc_task(task)` covers the
killed-server case: objects the dying task *wrote* on connections that belong
to other clients (a dead ne2kd's parked pushes) are evicted and its
`outstanding` charge refunded.

**There is no teardown notice opcode.** A closed connection makes the next
`ipc_send` to it answer `ConnectionNotFound` — that failure *is* the notice,
and it is the one every daemon already handles: ne2kd drops the watcher from
its fan-out list, inputd's `forget` evicts a dead watcher. Today that discipline
is dead code for killed clients (the send succeeds into the orphan mailbox);
the sweep makes it real. Adding a notice channel would put message semantics in
the kernel against the "kernel never interprets payloads" doctrine.

`MailboxManager::remove` grows an abandonment sweep: every future still parked
on the removed mailbox is consumed (through the notifier, so the registry and
`by_owner` stay honest) instead of evaporating from the waiters map. This is
the fix for the shell's UNWATCH-fail leak: `close()` disconnects, the mailbox
dies, the parked control receive dies with it. It also closes the whole class
for every program that disconnects over a pending receive.

`notify_waiters` prunes dead handles before matching messages: an IRQ-driven
`deliver_to_service` can interleave between sweep steps, and a pruned-but-
not-yet-freed handle must not consume a queued message (whose rightful receiver
is still waiting).

### 3.3 The reap (`kernel/src/kernel.rs`)

`reap_task_resources(task, spare_completion)` is the single teardown body,
called by `terminate_with_status` (with the task's own completion future as
the spared handle) and by `kill_task` (with the completion handle it was
addressed by — already stamped and notified before the reap starts). Order
inside the reap:

1. `discard_waiter(task)` — new on the voluntary path, symmetric to kill: an
   exited task must never sit in a waiter list even though the current wake
   discipline makes it unreachable today.
2. `close_connections_of(task)` — client-side teardown; mailbox abandonment
   consumes the parked receives inside it.
3. `gc_task(task)` — data objects it wrote on foreign connections + budget.
4. `discard_owned(task, spare)` — every remaining future it registered: time
   futures, waitsets, receives on long-lived binding mailboxes.
5. Prune mailbox-waiter and timer-queue entries naming the returned handles.

Then the existing steps (port revoke, group reclaim, scheduler removal)
continue unchanged. A voluntary exit is already `Terminated` and `current_task`
before the reap runs, so no wake path can resurrect it mid-sweep; a killed
victim follows the same order as the shipped kill path.

### 3.4 Known residue, deliberately kept

- The victim's **completion future** survives the sweep (its holder owns the
  report). If a parent dies while holding a child's completion handle, that
  one slot stays consumed-by-nobody — same as today, named in §6.
- **Bindings and IRQ subscriptions** carry no task identity at all; a dead
  daemon's binding and its `IRQ<n>` line outlive it. Reclaiming them needs a
  daemon-restart policy first — §6.

## 4. Tests

- **Future registry:** `discard_owned` frees time/waitset/message futures by
  owner and spares the named completion future; a parent waitset loses the
  edge and its pending events; `by_owner` stays honest across WaitFuture,
  WaitsetDrop and the sweep; generation-recycled handles never alias.
- **IPC:** `close_connections_of` tears down exactly the dying task's
  connections (a second client keeps working); `gc_task` evicts a dead writer's
  objects on foreign connections and refunds its charge; `mailbox.remove`
  consumes parked futures (the receive slot is gone, a later push delivers to
  the *next* parked future, not to a dead one); `notify_waiters` skips a dead
  handle without swallowing the message.
- **Kernel integration (unit):** killed sleeper — future gone, timer entry
  pruned, firing the timer afterwards is inert; killed receive-parker —
  connection closed, parked receive gone, the service's next send to it fails
  with `ConnectionNotFound`; voluntary `Exit` reaps identically to `Kill`
  (both paths through one body, both proven); the spared completion future
  still reports its stamped status to the holder after the reap.
- **Regressions:** the full `cargo test -p kernel -- --test-threads=1` (kill
  suite included), both-arch integration runs of `ctrl_c`, `exit_status`,
  `shell_background`, `boot_and_shell`, and `cargo xtask test --skip-integration`.

## 5. Milestones

| # | Change | Gate |
|---|---|---|
| M1 | this plan | — |
| M2 | future owner stamping + `by_owner` + `discard_owned` | kernel unit tests |
| M3 | `close_connections_of` + `gc_task` + mailbox-remove abandonment + `notify_waiters` prune | kernel unit tests |
| M4 | `reap_task_resources` on both death paths + handle pruning (mailbox, timer) | kernel unit tests |
| M5 | evidence log `plans/kill-resource-sweep-impl.md` + AGENTS.md | full gates: unit sweep, both-arch builds, `check-isa`, integration regressions |

## 6. Non-goals (named follow-ups)

- **`kill <job>` in the shell** (kill a background task by name) — the sweep
  removes the resource hazard for it, but the command itself is its own plan.
- **Daemon identity**: reclaiming a dead daemon's `IpcBind` binding and
  `IrqSubscribe` line — needs a restart policy, and kstatsd binds directly
  inside the kernel, so the rule must not break the kernel-hosted pattern.
- **A `FutureDrop` syscall** — with mailbox abandonment and the sweep there is
  no caller left for it; the no-test-only-surface rule forbids adding it as
  theory.
- **Orphan-of-a-dead-holder completion futures** (§3.4) and the
  `ipc-volume-wedge` silent-death diagnosis — adjacent hygiene, different root.
