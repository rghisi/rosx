# Adaptive arenas — paged growth, folded future side tables, measured before and after

> **Date:** 2026-11-22 · **Branch:** `ccr-3dffef28-eeqbct` · **Base:** `origin/main@7c4a5f1`
> **Status:** proposal, approved in discussion; steps land one commit each.
> **Evidence base:** a memory-footprint discovery item ("shrink eager arenas (start at 32 slots and
> grow) or compact the future tables (`parents_of` at 24 B × 1024, `owners` at 16 B × 1024)") and a
> read of `kernel/src/collections/arena.rs`, `kernel/src/future/mod.rs`, `kernel/src/handle.rs`.
> Every byte figure below is an estimate from reading the code; Step 0 produces the real numbers.

## Goal

`GenerationalArena` keeps O(1) `add`/`borrow`/`remove` but starts small, grows on demand and has no
fixed-size hard limit baked into its type. The eager footprint of the future registry (arena plus the
two parallel tables, about 60 KB today) and of the five 256-slot arenas drops to roughly one page each.

## Today

- `GenerationalArena<T, const S>` preallocates `items: Vec<Option<T>>`, `generations: Vec<u16>` and a
  `VecDeque<u16>` free list, all S long. A full arena returns `Error::OutOfMemory`, so S is both the
  initial footprint and a hard quota.
- Six users: futures (1024), tasks, data objects, bindings, connections and mailboxes (256 each).
- `FutureRegistry` keeps `parents_of: Vec<Vec<ParentLink>>` (24 B per slot) and
  `owners: Vec<FutureOwner>` (16 B per slot), both sized `FUTURE_SLOTS` and indexed by arena slot, so
  they must stay in lockstep with the arena by hand.
- `reclaim_group` scans every slot of `owners`: O(capacity).
- `Handle` is a `u16` index plus a 15-bit generation, so no arena can exceed 65 536 slots.
- The free list is FIFO, which maximises the distance before a slot is reused.

## Decisions taken

1. **Paged arena:** a directory of fixed 32-slot pages, allocated on demand. Lookup is two loads,
   growth costs O(32) and copies nothing, and no large contiguous block is ever needed.
2. **Intrusive FIFO free list** with the generation stored in the slot, replacing the separate
   `generations` Vec and the `VecDeque`. Head and tail indices keep reuse FIFO and O(1).
3. **Cap is a constructor argument.** The const generic S goes away. Every arena is capped at 1024
   slots for now; the cap is tuned per subsystem later. `add` returns `OutOfMemory` at the cap or when
   a page allocation fails, and never panics on growth failure.
4. **Generation wrap is documented, not mitigated.** A slot reused 32 768 times can match a stale
   handle again; no slot retirement.
5. **Shrinking is deferred** and recorded as an open decision.
6. **Fold `owners` and `parents_of`** into one per-future entry so the registry has no parallel
   tables. Most futures have no parents, so `parents` is a thin optional boxed slice or a sparse map.
7. **`by_group` index** replaces the `reclaim_group` scan, as its own commit and the lowest priority:
   with a 1024 cap the scan is cheap, the value appears when caps rise.
8. **The 16-bit index and 15-bit generation split stays.** Changing it is an ABI change in `system`
   and needs approval first; 65 535 slots are enough for every foreseen arena.
9. **Measure before and after** with disposable, uncommitted harness code (Step 0 and Step 6).

## Measurement protocol (Steps 0 and 6)

The harness is disposable. It lives as an uncommitted `#[cfg(test)]` module in `kernel/` (the arena API
is `pub(crate)`), kept as a patch and result files in the scratchpad. `check-style` surveys
`git ls-files` only, so untracked scratch cannot trip a gate and nothing test-only lands in production.
A thin adapter hides the API difference (const generic before, constructor argument after) so the same
workload code runs on both commits.

**Footprint (deterministic, the stronger evidence).** A counting `#[global_allocator]` wrapper reports:

- bytes allocated right after constructing `FutureRegistry`, `IpcManager`, `DataManager`,
  `MailboxManager` and `TaskManager` (the eager boot cost);
- bytes at 0, 32, 100 and 1024 live entries per arena (the growth curve);
- `size_of` for the slot and entry types.

**Speed (relative only).** `std::time` loops with many iterations, median of several runs:

- `add`, `borrow`, `remove` at low, medium and full occupancy;
- churn (interleaved add and remove) to exercise the free list;
- worst single `add` latency across a growth event;
- `reclaim_group` with few and with many futures.

**Real boot footprint (optional).** If `kstatsd` exposes heap usage, compare QEMU boot numbers per arch
before and after. Check what `kstatsd` reports first; this is the only number that reflects the real
allocator and the 32-bit layout.

Preconditions for every timing run: `uptime` load against `nproc`, and
`pgrep -f 'cargo test|xtask test|qemu-system'` empty. Host timings are noisy and say nothing about a
486; byte counts decide, timings only show relative cost.

## Step 0 — baseline on the unchanged tree

- Write the harness and adapter against the current API, run the full protocol on `7c4a5f1` and save
  the results.
- Record them in `plans/adaptive-arenas-results.md` (tables: eager bytes per type, growth curve,
  timings) and commit that document only. The harness itself stays uncommitted.
- If the optional boot footprint applies, capture it on both arches here.
- End with the `retro` skill's step-end mode.

## Step 1 — paged arena

- Rework `kernel/src/collections/arena.rs`: `GenerationalArena<T>` with a page directory, 32-slot
  pages, intrusive FIFO free list, constructor `new(max_slots)`. Assert `max_slots` is in
  `1..=65535`. The public surface (`add`, `borrow`, `borrow_mut`, `remove`, `live_generation`, `any`,
  `visit`) is unchanged, and `visit`/`any` skip empty pages.
- Update the six users to pass the 1024 cap; `FUTURE_SLOTS` becomes the futures cap.
- Unit tests replace the `items.len() == S` assertions: initial capacity of one page, growth across
  page boundaries, handles stable across growth, cap and `OutOfMemory`, FIFO reuse order, stale-handle
  rejection after remove, generation advance, `visit` and `any` over multi-page contents.
- The side tables of `FutureRegistry` are still the eager ones here; their sizing now derives from the
  cap and is removed in Step 2.
- Gate: `cargo xtask check`.

## Step 2 — fold `owners` and `parents_of` into the future entry

- Introduce a per-future entry (`future`, `owner`, `parents`) stored in the arena. Remove `owners` and
  `parents_of` and the `empty_owner_table`/`empty_parent_rows` helpers, and rewrite their accesses
  (`register`, `consume_inner`, `cascade_to_waitsets`, `reclaim_group`, link and unlink paths).
- Behaviour must not change; the existing registry unit tests that read `parents_of` and `owners`
  directly move to the entry accessors.
- The FUTURE_SECTION critical-section discipline and the "dying task's futures are reaped on both
  `Exit` and `Kill`" invariant are untouched.
- Gate: `cargo xtask check`, then one `cargo xtask test` run on the host used for the baseline
  (futures are on every boot path).

## Step 3 — `by_group` index for `reclaim_group`

- Add `by_group: BTreeMap<GroupId, Vec<FutureHandle>>` maintained on `register`, `consume_inner` and
  `reclaim_group`, so reclaim is O(live futures of the group) instead of a scan of the arena.
- Tests: reclaim frees only that group's futures, leaves other groups' and waitsets alive, and the
  index is empty after the last future is consumed.
- If it makes the registry noticeably harder to follow, drop this step and record a
  `docs/todo/` item instead.
- Gate: `cargo xtask check`.

## Step 4 — records and docs

- `docs/decisions/arena-shrinking-deferred.md` (`status: open`, `revisit-when`: boot-time peak and
  steady-state usage diverge noticeably, or a subsystem cap above 1024 lands).
- `docs/decisions/arena-generation-wrap-accepted.md` (`status: decided`): wrap after 32 768 reuses of
  one slot, no retirement, why.
- Update `docs/architecture/ipc-and-futures.md` and `docs/architecture/memory.md` with the new arena
  invariants: paged growth, per-arena cap as a quota, no parallel tables.
- Commit.

## Step 5 — final measurement and report

- Re-run the Step 0 protocol on the final head with the same harness and host conditions.
- Append the after tables and the before/after delta to `plans/adaptive-arenas-results.md`, with a
  short reading: eager bytes saved, growth curve, whether any timing regressed.
- Gate: `cargo xtask check`; then one `cargo xtask verify` on a quiet host, because the change touches
  boot on both arches.
- End with the `retro` skill's step-end mode.

## Risks

- **Quota lost.** Removing the fixed type-level limit must not let a task exhaust the kernel heap. The
  1024 cap preserves today's ceiling for every arena; a later per-subsystem or per-group quota is a
  tuning decision.
- **Allocation failure on grow** inside a syscall with IF=0: `add` must surface `OutOfMemory`, and
  callers already handle it.
- **Generation wrap window** is shorter with a smaller working set; accepted, recorded in Step 4.
- **Host timing noise:** decided by bytes, not nanoseconds.
- **Index behaviour change:** indices are now handed out from a growing set and FIFO order is kept, so
  tests that assumed specific indices from a preallocated arena need review.

## Out of scope

Shrinking and freeing empty pages, a hierarchical-bitmap free finder, per-subsystem cap tuning,
per-group quotas, any change to the `Handle` bit layout, and an O(1) `by_owner` removal (the current
`retain` is linear per consume). Each is a follow-up once the Step 5 numbers exist.

## Friction log

Step 0 | harness first compile: `FutureHandle` imported from `crate::future` (private re-export) and `&core::any::Any` missing `dyn` in the scratch module | TASK | 1 round
Step 0 | harness first run: 3 of 5 bench tests panicked (churn kept 1 live per 3 adds so 2 000 > 1 024 cap; connections curve needs the 257th mailbox slot the service binding itself takes) | TASK | 1 round
Step 1 | first arena test run: 9 failures — reuse-distance assumptions wrong in both directions (a freed slot queues behind the page's untouched slots, and the old fill-to-`FUTURE_SLOTS` registry tests recycle slot 0 mid-fill now) | TASK | 2 rounds
Step 1 | `use core::alloc::alloc` unresolved on the pinned nightly (raw fns live in `alloc::alloc` for the no_std kernel) and `Layout::array_of` is `Layout::array` there | ENV-TRAP | 1 round
Step 2 | test-rewrite script aborted on its own leftover-guard: the test name `reclaim_purges_parents_of_rows...` matched the grep for `parents_of`, so the rewrite never wrote; script rerun | TASK | 0

