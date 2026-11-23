# Kernel BTreeMap removal — fold, delete or flatten every map, measured before and after

> **Date:** 2026-11-22 (revised 2026-11-23) · **Branch:** `ccr-8c760afd-pvgqik` · **Base:** `origin/main@f85af9f`
> **Status:** proposal; steps land one commit each.
> **Depends on:** `plans/adaptive-arenas.md`. All its steps have landed (#202): paged arena with the
> generation stored in the slot, `owners` and `parents_of` folded into one `FutureEntry`, and a
> `by_group` index. That `by_group` index is a `BTreeMap`, and this plan **removes it**; see Decisions.
> **Evidence base:** a memory-footprint assessment item ("remove BTreeMap duplication: 7 distinct
> instantiations take 54.6 KB of x86_64 `.text` (34%) and 50.1 KB on x86_32; replace the handle-keyed maps
> with slot-indexed tables") and a read of `kernel/src/future/`, `kernel/src/ipc/`,
> `kernel/src/scheduler/`. The assessment document itself is not in the repo, so its byte figures are
> unverified here. Every other figure below is an estimate from reading the code; Step 0 produces the
> real numbers.

## Goal

No `BTreeMap` (or `BTreeSet`) remains in non-test `kernel/` code, so the generic B-tree node code
disappears from both kernels, and no operation in the kernel gets worse than a bounded scan of at most
1024 occupied slots. Hot paths stay O(1) or improve.

## Today

Ten map fields in `kernel/src`, each a distinct `(K, V)` instantiation of the B-tree code:

| # | Field | Key → value | What it really is |
|---|---|---|---|
| 1 | `FutureRegistry.waiters` | FutureHandle → Vec<TaskHandle> | tasks blocked on a future |
| 2 | `FutureRegistry.by_owner` | TaskHandle → Vec<FutureHandle> | reverse index of `future.owner()` |
| 3 | `IpcDataManager.outstanding` | TaskHandle → usize | per-writer byte budget |
| 4 | `IpcDataManager.by_connection` | IpcConnectionHandle → Vec<Handle> | reverse index of `DataObject.connection` |
| 5 | `IpcManager.by_client` | TaskHandle → Vec<IpcConnectionHandle> | reverse index of `IpcConnection.client_task` |
| 6 | `IpcManager.registry` | String → IpcBindingHandle | service name lookup |
| 7 | `MailboxManager.waiters` | Handle → Vec<FutureHandle> | receivers parked on one mailbox |
| 8 | `MlfqStrategy.yield_info` | TaskHandle → YieldInfo | scratch between `record_yield` and `requeue_after_run` |
| 9 | `TimerManager.next` | u64 → Vec<FutureHandle> | sleepers ordered by deadline |
| 10 | `FutureRegistry.by_group` | GroupId → Vec<FutureHandle> | reverse index of `FutureEntry.owner` (added by #202) |

Six of them (2, 4, 5, 7, 10, and the by-key half of 1) are derived data whose source of truth already sits in
an arena entry. Maintaining them is what makes `evict`, `consume_inner` and `disconnect` linear today.
`scheduler.rs` also has a `BTreeMap` inside `#[cfg(test)]`; it is out of scope.

## Decisions taken

1. **No slot-indexed side tables.** After the adaptive-arenas plan the capacity is a runtime cap of 1024
   per arena, so a table sized to the cap costs 24 KB per `Vec<Vec<_>>` on x86_64 and defeats that plan's
   purpose. Data lives inside the arena entry (generation-validated for free), is derived by scanning
   with `visit`/`any` (which skip empty pages), or is a small sparse `Vec`.
2. **Waiters fold into the future entry.** `FutureEntry` already holds `future`, `owner: FutureOwner`
   and `parents: Option<Vec<ParentLink>>` (56 B per slot on x86_64). One `links: Option<Box<Links>>`
   field replaces `parents` and takes `waiters` too, so the entry **shrinks** (24 B → 8 B on x86_64)
   and only futures that have a waiter or a parent allocate. The entry also caches the owning
   `TaskHandle` (4 B), so a scan reads it without a virtual call.
3. **`by_owner`, `by_group`, `by_connection`, `by_client` are deleted, not replaced.** Reclaim paths scan
   the owning arena with `visit`. Per-task teardown batches the data-object sweep into one pass.
4. **Mailbox waiters move into `Mailbox`.** The parked-receiver list is a property of the mailbox.
5. **Timer is an `alloc::collections::BinaryHeap` (available in `no_std`, no new dependency)** of
   `Reverse(entry)`, where the entry's `Ord` compares only `(deadline, seq)` and carries the
   `FutureHandle` as payload. `seq` is a `u64` counter that keeps same-deadline wakeups FIFO. The timer is
   a priority queue, not a map: it needs insert, pop-everything-due and discard-by-handle. Deadlines are
   nearly monotonic, so a plain binary search tree would degenerate and a balanced one costs too much code.
   A standalone measurement (host x86_64, opt-level 2) put one `BTreeMap` instantiation at about 14 KB of
   `.text` against about 1.5 KB for the heap timer; the figure is indicative only, Step 0 measures the
   kernel.
6. **Registry is a sorted `Vec<(String, IpcBindingHandle)>`** with binary search.
7. **`yield_info` and `outstanding` are sparse `Vec`s keyed by `TaskHandle`**, scanned linearly or by
   binary search. `yield_info` is not collapsed into a single `Option`: production code records and
   requeues back to back (`scheduler.rs`), but that adjacency is not part of the `SchedulingAlgorithm`
   contract and the unit tests record two handles before requeueing.
8. **`by_group` goes too, and `reclaim_group` becomes a scan.** #202 added the index so that
   `reclaim_group` is a lookup, O(k). Keeping it would leave a `BTreeMap` linked, which defeats the goal.
   The scan is O(slots in materialized pages), see the next point. This is a **regression** against
   `main` and the complexity tables say so. Record a `docs/todo/` item naming an intrusive
   `next_in_group` link (with Step 1b) as the way back to O(k).
9. **Scan latency is bounded and recorded.** `visit` and `any` walk every slot of every materialized
   page. Pages are never freed (`docs/decisions/arena-shrinking-deferred.md`), so a scan costs the
   arena's **high-water mark**, not its live count: after one burst of 1024 futures it stays 1024.
   Reap paths run with IF=0. The bound is 1024 slots for futures (256 for the IPC and task arenas); the
   decision record `reap-scan-latency-budget` is opened with `revisit-when` set to "a measured reap on
   x86_32 hardware or emulation exceeds 1 ms, or any arena cap rises above 1024".

## Complexity analysis

Notation: `F` live futures, `Fw` futures with at least one waiter, `W` total waiters, `w` waiters of one
future, `k` futures owned by one task, `T` tasks, `C` live connections, `c` connections of one task,
`D` live data objects, `d` objects of one connection, `Wr` writers with outstanding bytes, `M` mailboxes, `g` futures of one group,
`B` bound services, `L` service-name length, `S` pending sleepers, `k_t` expired timers, `Y` outstanding
yield records, `p` materialized pages of the arena being scanned (its high-water mark; pages are never
freed). A scan visits `32·p` slots, with `F ≤ 32·p ≤ 1024`. Each cell is worst case; amortised pushes and the cost of the map nodes' allocation are
listed separately under "allocations".

### Hot paths

| Operation | Today | After | Note |
|---|---|---|---|
| `FutureRegistry::register` | O(log T) + amortised O(1) | **O(1)** | no `by_owner` entry |
| `register_waiter` | O(log Fw) | **O(1)** | direct entry access; first waiter allocates the `Links` box |
| `notify_inner` (wake) | O(log Fw + w) | **O(1 + w)** | take `links.waiters` |
| `consume_inner` | O(log T + k) | **O(1)** + parents | the `retain` over the owner's futures is gone; consuming all of a task's futures was O(k²) |
| `IpcDataManager::write` | O(log Wr + log C) | **O(log Wr)** | binary search over a flat `Vec`; one fewer tree update |
| `IpcDataManager::release` | O(log Wr + C + D) | **O(log Wr)** | `evict` today walks every `by_connection` list |
| Mailbox park / `push_back` wake | O(log M) | **O(1)** | the mailbox is already borrowed |
| `MlfqStrategy::record_yield` / `requeue_after_run` | O(log Y) | **O(Y)**, Y ≤ 1 | production: no lookup cost |
| `TimerManager::pop_expired`, every scheduler step | O(log S + min(k_t, S − k_t)) | **O(1)** when nothing is due, else **O(k_t · log S)** | `peek` is a single comparison |
| `TimerManager::add_sleep` | O(log S) | **O(1)** amortised for increasing deadlines, O(log S) worst | allocation-free once warm; no shifting |
| `IpcManager::connect` | O(L log B + log T) | **O(L log B)** | no `by_client` entry |

No hot-path operation has a worse worst case than today. The heap pays O(log S) per expired timer
instead of the tree's bulk split, which only matters when many sleepers expire in one step.

### Teardown and cold paths

| Operation | Today | After | Note |
|---|---|---|---|
| `discard_waiter(task)` | O(Fw + W) | O(32·p_f + W) | **regression**: visits every future slot instead of only futures with waiters |
| `discard_owned(task)` | O(log T + k·log Fw) | O(32·p_f + k) | scan replaces the index; the per-future wake-list removal is O(1) |
| `reclaim_group` | O(g · log F) (since #202) | O(32·p_f + g) | **regression** against `main`: the `by_group` lookup becomes a scan |
| `IpcDataManager::gc_task` | O(D + k·(C + D)) | O(32·p_d + k) | today each `evict` rescans all lists |
| `gc_connection` / `disconnect` | O(log T + c + d·(C + D)) | O(32·p_d + d) | `by_client` upkeep gone |
| `close_connections_of(task)` | O(log T + c·(…) + D) | O(32·p_c + 32·p_d + c) | batched: one pass over data objects, one over connections |
| `MailboxManager::remove` | O(log M + w) | O(w) | |
| `discard_futures` (mailbox) | O(M_w·w·m) for m orphans | O(32·p_m + Σw·m) | outer loop visits every mailbox slot, not only those with waiters |
| `discard_futures` (timer) | O(S·m) | O(S·m + S) | `retain` then heap rebuild; cold path |
| `bind_service` | O(L log B) | O(L log B) + O(B) shift | boot-time only |
| `on_task_terminate` | O(queue length) | O(queue length) | unchanged |

Reading:

- **Asymptotic wins** come from removing the maintenance of derived data. The worst offender is
  `evict`, called per data object, which scans every `by_connection` list. The `retain` in `consume_inner`
  made a task teardown quadratic in its own futures.
- **Regressions** are the by-key lookups that became scans (`discard_waiter`, `discard_owned`, the mailbox
  `discard_futures`, and `reclaim_group` since #202). They occur only on task exit and kill, and are
  bounded by 1024 slots. A system that never bursts keeps `p` at 1–3 pages (32–96 slots). A system that
  once held 1000 futures scans 1024 slots on every later exit, because pages are never freed. Step 0
  measures both cases.
- **Allocations (IF=0 matters).** Today every first future of an owner allocates a `Vec` in `by_owner`, and every
  first future of a group one in `by_group` (freed again when the last one is consumed), and each first insert into a map can allocate a
  node. After: `register` allocates nothing, `register_waiter` allocates one `Links` box per future that
  gains a waiter or parent, and the timer and sparse `Vec`s stop allocating once warm.
- **Space.** Eager cost is zero: no table is sized to the cap. Per future entry, −16 B (x86_64) for `links`
  replacing `parents` (24 B → 8 B), plus 4 B for the cached task owner. The `BTreeMap` node overhead
  (internal and leaf nodes, per-entry `Vec` headers) is removed.
- **Ordering.** `discard_owned` used to return futures in registration order and will return them in slot
  order. The caller (`reap_task_resources`) only forwards the list, but tests that assert order need review.

## Measurement protocol (Steps 0 and 6)

Reuse the disposable harness approach of `plans/adaptive-arenas.md`, and extend it:

- **Code size (the headline number).** `nm --size-sort -S` on each release kernel ELF
  (`cargo xtask build x86_64`, `cargo xtask build x86_32`): total `.text`, and the summed size of symbols
  matching `btree`/`BTreeMap`. Record per arch before and after every code step.
- **Allocation counts per operation.** The counting `#[global_allocator]` wrapper reports allocations for
  `register`, `register_waiter`, `consume`, `write`, `release`, `connect`, `disconnect`, `add_sleep`,
  `pop_expired`.
- **Scan budget.** For `discard_waiter`, `discard_owned`, `close_connections_of`, count slots visited at
  1, 32, 100 and 1024 live entries, and time with `std::time` as a relative figure only.
- **Throughput.** Loops over `register`+`consume`, `write`+`release`, `add_sleep`+`pop_expired` at low,
  medium and full occupancy.

Same preconditions as the adaptive-arenas plan: quiet host (`uptime` against `nproc`; no
`pgrep -f 'cargo test|xtask test|qemu-system'` hits). Bytes decide; host timings are relative only.

## Step 0 — baseline

- Adaptive arenas has landed. Reuse `plans/adaptive-arenas-results.md` (boot `used` 1696 KiB x86_64,
  1455 KiB x86_32; eager and growth curves; timings) and add the `.text` and allocation-count tables
  above, measured on the current `origin/main` head.
- Add a scan-budget row for a **post-burst** arena: fill 1024 futures, free them, keep 3 live, then time
  `discard_owned`, `discard_waiter` and `reclaim_group`.
- Record them in `plans/kernel-btreemap-removal-results.md` and commit only that document.
- End with the `retro` skill's step-end mode.

## Step 1 — future registry

- Replace `FutureEntry.parents` with `links: Option<Box<Links>>` (holding `waiters` and `parents`); cache
  the task owner in the entry; delete `waiters`, `by_owner` and `by_group`.
- `discard_waiter`, `discard_owned` and `reclaim_group` scan with `visit`, collect handles first, then mutate (the borrow
  rules and the existing `discard_owned` spare-handle behaviour stay as they are).
- Tests: register, wake and consume with and without waiters; `discard_waiter` removes a task only from
  waiters; `discard_owned` with and without a spare completion; an empty `Links` box is freed when its last
  waiter and parent are gone; existing registry tests move to the entry accessors (those that read `registry.waiters`, `by_owner` and
  `by_group` directly, for example `reclaim_group_*` and `register_indexes_every_future_by_owner`, are
  rewritten to observable behaviour); `reclaim_group` frees only its group's futures after a burst.
- Gate: `cargo xtask check`; one `cargo xtask test` (futures are on every boot path).

## Step 2 — IPC and mailbox

- Delete `by_connection` and `by_client`; `gc_connection`, `gc_task` and `close_connections_of` become
  scans, with the per-task teardown batched into one pass over data objects and one over connections.
- Move the parked-receiver list into `Mailbox`; `discard_futures` scans mailboxes.
- Tests: disconnect frees only that connection's objects and refunds the right writers; task exit closes
  every connection it opened and sweeps objects it wrote; parked receivers are abandoned on `remove`;
  `release` no longer touches other connections.
- Gate: `cargo xtask check`.

## Step 3 — timer and service registry

- `TimerManager` as a `BinaryHeap<Reverse<TimerEntry>>` with a `(deadline, seq)` `Ord` that ignores the
  handle; `registry` as a sorted `Vec` with binary search.
- Tests: FIFO order among equal deadlines, nothing due returns `None` without allocating, partial expiry,
  `discard_futures` keeps the heap valid and the order of the survivors, duplicate service names still
  rejected, lookup of a missing name.
- Gate: `cargo xtask check`.

## Step 4 — scheduler and budget scratch

- `yield_info` and `outstanding` become sparse `Vec`s (`outstanding` sorted, binary search; entry removed
  at zero).
- Tests: the two-records-before-requeue case, termination with a pending record, budget refund to zero.
- Gate: `cargo xtask check`.

## Step 1b — optional: intrusive per-owner list

Only if the Step 0 and Step 1 numbers show the `discard_owned` or `reclaim_group` scan is too slow (see the
`reap-scan-latency-budget` record); otherwise skip and delete this step from the plan.

- Thread a doubly linked list through the future entries (`u16` prev and next, 4 B per entry) with a
  small sparse `Vec` of per-task heads; a second list can do the same per group, restoring O(g) for
  `reclaim_group`. `discard_owned` becomes O(k), `consume_inner` stays O(1), and no
  tree or map comes back.
- Cost: an index to keep in sync again, which is the risk this plan otherwise avoids.
- Tests: link and unlink at head, middle and tail; consume of a future in the middle; teardown with a spare
  completion; list empty after the last future is consumed.
- Gate: `cargo xtask check`.

## Step 5 — cleanup, records, docs

- Remove `Ord`/`PartialOrd` derives from the handle types and `GroupId` (derived for `by_group` in #202) if nothing else needs them; confirm with
  `grep -rn BTree kernel/src` that only the `#[cfg(test)]` mock remains.
- `docs/decisions/reap-scan-latency-budget.md` (`status: open`), and the `docs/todo/` item for the
  intrusive per-group list that would restore O(g) `reclaim_group`.
- `docs/architecture/ipc-and-futures.md`: no reverse indexes for data an arena entry already holds;
  teardown scans occupied pages; `docs/architecture/memory.md`: no table sized to a cap.
- Commit.

## Step 6 — final measurement and report

- Re-run the Step 0 protocol on the final head under the same host conditions; append before/after tables
  to the results file: `.text` per arch, allocation counts, scan budget, throughput.
- Gate: `cargo xtask check`, then one `cargo xtask verify` on a quiet host (futures, IPC and the scheduler
  are touched on both arches).
- End with the `retro` skill's step-end mode.

## Risks

- **Scan latency under IF=0.** Bounded by 1024 slots per scan (the arena's high-water mark, not the live count), a few microseconds on a modern host and
  possibly around a millisecond on a 486 (unmeasured). Mitigation: the Step 6 numbers and the
  `reap-scan-latency-budget` record; the follow-up is the optional Step 1b intrusive per-owner list, not a
  re-introduced map.
- **Allocation failure when a `Links` box is created** inside `FUTURE_SECTION`. `register_waiter` has no
  error path today either (a map insert would fail the same way); recorded, not changed here.
- **Behaviour assumed by tests**: `discard_owned` order, FIFO among equal deadlines (the heap's `seq`
  tiebreak) and among waiters (`Vec::remove(0)` stays), indices handed out by the paged arena.
- **Heap growth** allocates and panics on out-of-memory like `Vec`; this is the same failure mode as a
  `BTreeMap` insert today.
- **Borrowing while scanning**: handles are collected first and mutated after, as `gc_task` does today.
- **Size claim unverified**: if Step 0 shows the B-tree symbols are far smaller than the assessment's
  figure, re-evaluate before spending Steps 1 to 4 (Step 3 is the cheapest to land first).

## Out of scope

Shrinking or freeing empty pages (`arena-shrinking-deferred`), an O(1) `discard_waiter` (a per-task `waiting_on` field), turning
the mailbox waiter `Vec` into a `VecDeque`, and the `BTreeMap`s in user-space
crates (`vfsd`, `ramfs`, `rxfs`), which are separate binaries.
