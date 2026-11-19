# Kill-aware resource sweep — implementation evidence log

Branch: `kill-cleanup` (stacked on `ctrlc-interrupt`, PR #176). Parent document:
`plans/kill-resource-sweep.md`. Running log of every milestone gate: what was
run, what it printed, what it proved. Host machine: QEMU 10.2.1, rustc
1.101.0-nightly `0abfedbc7` 2026-10-02.

## Decisions taken (per the plan's recommendations)

- **D1:** one reap on **both** death paths, not a kill special case —
  `Kernel::reap_task_resources(task, spare_completion)`.
- **D2:** ownership rides on the future object (`Future::owner`, every kind
  constructed `owned_by`), the registry keeps the `by_owner` index —
  `ForNotifyingFutures::register` keeps its signature.
- **D3:** no teardown-notice opcode; the closed connection making the next
  `ipc_send` answer `ConnectionNotFound` *is* the notice, consumed by the
  eviction arms daemons already wrote (ne2kd fan-out, inputd `forget`).
- **D4:** the victim's completion future is spared by the sweep — its holder
  still consumes it and collects the exit status.

## Refinements taken during implementation

- `FutureRegistry::replace` and `GenerationalArena::replace` were **deleted**:
  they had no caller left, and a replace-under-the-same-handle contradicts the
  `by_owner` index (the replacement's owner would desync).
- `discard_owned` needed a real production caller to obey the
  no-test-only-surface rule, so it landed wired into `kill_task` in M2 and M4
  only factored both paths into the shared body.
- The reap's `spare_completion` is an `Option` — the voluntary path reads the
  completion handle from `TaskManager` (a task created without pairing has
  none) and hands the same handle it notified.
- `pop_expired(now)` computes `now + 1` and overflowed the debug test at
  `now = u64::MAX`; the system clock cannot reach it (milliseconds) — noted,
  not a fix in scope; the test pops at a plain past-deadline.
- Mailbox-remove abandonment and the `notify_waiters` dead-handle prune landed
  together in M3: the retain is what keeps a handle consumed mid-sweep (an IRQ
  can interleave `deliver_to_service`) from swallowing a queued message.

## M2 — future ownership + `discard_owned` (`72b97b2`)

- Edits: `Future::owner` on the trait; `TimeFuture::owned_by`,
  `WaitsetFuture::owned_by`, `IpcMessageFuture::owned_by/with_message/with_error`
  all take the owner; `FutureRegistry` gained `by_owner`, register indexes,
  `consume_inner` prunes (empty rows disappear); `discard_owned(task, spare)`
  consumes the owner's futures (detaching waitset edges via the existing
  path, releasing parked-waiter entries) and returns the freed handles;
  wired into `kill_task`.
- Gate: `cargo test -p kernel -- --test-threads=1` — **440 green** (was 433).
  New tests pin: register indexes by owner in insertion order; consume prunes
  and empty rows disappear; `discard_owned` frees everything but the spared
  completion and leaves a bystander untouched; the spare stays indexed and its
  later consume leaves no row; a victim child is detached from a foreign
  waitset (edge and `parents_of` row); a parked waiter entry is released.

## M3 — connections, data, mailbox abandonment (`de5587f`)

- Edits: `IpcConnection` enumeration through `IpcManager::by_client`
  (maintained by connect/disconnect); `close_connections_of(task)` tears down
  exactly the dying task's client connections and then `IpcDataManager::gc_task`
  evicts what it **wrote** on connections that outlive it (refund included);
  `MailboxManager::remove` abandons the futures parked on the removed mailbox
  (new `ForNotifyingFutures::abandon` → registry consume); `notify_waiters`
  prunes dead handles before matching (new `is_live`); `pop_front_async`,
  `accept_message_async` and `receive_message_async` take the owner/caller.
  This closes the shell's UNWATCH-fail leak (deferred item 3) without touching
  the shell: `close()`/`disconnect` now consumes the parked control receive
  with the mailbox, on the send-fail path and the early-bail paths alike.
- Gate: `cargo test -p kernel -- --test-threads=1` — **450 green**. New tests
  pin: `close_connections_of` answers `ConnectionNotFound` on the dead client's
  connection while a second client keeps working; parked receives are consumed;
  the written budget is refunded (4×64 KiB re-writable after death); a dead
  writer's object on a foreign live connection is evicted and the connection
  stays usable; `gc_task` refunds a full budget; mailbox `remove` consumes
  parked futures; a dead parked handle is pruned without swallowing the pushed
  message (the next receiver gets it); kill-level: the victim's connection
  answers `ConnectionNotFound`, its parked receive is gone.

## M4 — the shared reap (`392c12e`)

- Edits: `Kernel::reap_task_resources` is the single body — `discard_waiter`
  (now on the voluntary path too) → `close_connections_of` → `discard_owned` →
  prune mailbox parked lists and the timer queue by returned handle identity
  (`MailboxManager::discard_futures`, `TimerManager::discard_futures`);
  `terminate_with_status` and `kill_task` both run it before
  `portio::revoke` → group reclaim → scheduler removal, keeping the shipped
  stamp-before-teardown ordering intact on both.
- Gate: `cargo test -p kernel -- --test-threads=1` — **454 green**. New tests
  pin: timer `discard_futures` removes named handles and drops emptied
  deadlines; mailbox parked lists are pruned by handle identity (a survivor
  still receives); the reap body end-to-end — parked receive gone, sleep future
  gone, timer queue clean, connection `ConnectionNotFound`, and the spared
  completion still reports its stamped status to the holder.

## Final gates (both arches)

- `cargo test -p kernel -- --test-threads=1` — 454 green.
- `cargo xtask test --skip-integration` — 61 result tables, zero failures.
- x86_64: `cargo xtask build` green (kernel, 41 apps, ROM, FAT boot disk);
  `ROSX_KERNEL_ELF=$PWD/target/rosx/release/rosx cargo test -p
  tests-integration -- --test-threads=1` per suite:
  - `ctrl_c` 4/4 (62.9 s), `exit_status` 5/5 (78.2 s),
    `shell_background` 5/5 (77.5 s), `boot_and_shell` 4/4 (47.9 s),
    `shell_command_arguments` 1/1, `waitset_or` 1/1, `ipc_bulk` 1/1.
- x86_32: `cargo xtask build x86_32` green; `cargo xtask check-isa x86_32` —
  "kernel and 41 user ELFs are 486-clean"; `ROSX_ARCH=x86_32` per suite:
  `ctrl_c` 4/4 (60.6 s), `exit_status` 5/5 (77.7 s),
  `shell_background` 5/5 (74.8 s).

## What a dying task now leaves behind

Nothing it owned. Futures (time, receive, waitset), client connections, client
mailboxes, parked receives, data objects and their budget charge, timer-queue
and mailbox-waiter edges — all reaped on `Exit`, `Kill` and the panic-handler
redirect alike. Named residue that stays, deliberately:

1. **The spared completion future** lives until its holder consumes it; a
   holder that dies holding another task's completion handle leaves that one
   slot (same as before this PR; §3.4 of the plan).
2. **Bindings and IRQ subscriptions** carry no task identity — a dead daemon's
   `"SERVICE"` binding and its `IRQ<n>` line outlive it; reclaim needs a
   restart policy and must not break kernel-hosted kstatsd. Follow-up plan.
3. The `ipc-volume-wedge` silent-death diagnosis is untouched — this is
   hygiene, not that bug.
