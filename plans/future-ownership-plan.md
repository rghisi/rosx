# Group-Owned Futures — closing the registry leak class

> **Status:** Proposed. Written 2026-11-15 at worktree `misty-nebula`, HEAD `adbcedf`.
> **Goal:** every future-registry slot has an owning loaded-image group, and the kernel reclaims a
> group's unconsumed slots when its last live member terminates — making registry exhaustion
> unreachable by abandonment, the way `BlockOwner::Group` makes memory leaks unreachable.
> **Method:** every code fact below is cited to a path (and line) re-verified against this tree;
> runtime facts are marked **[O]**. Facts this plan cannot prove are marked **unverified** and
> routed to the milestone that must prove them.
> Repo law applies throughout: `kernel/` stays `no_std` and arch-agnostic, **no comments ever**
> (assembly excepted), unit tests for kernel modules, no test-only production surface,
> `cargo build --workspace` stays forbidden, commit subjects are bare imperative (no prefix).

---

## 0. Goal & Non-Goals

### Goal
- The future registry is the last kernel resource without an ownership story (memory has groups,
  ports have grants, futures have tokens and hope). Give it one, mirroring
  `memory ownership = loaded-image groups`.
- Abandonment can no longer leak a slot past the lifetime of the group that abandoned it: a dying
  group forfeits its unreaped reports, and the slots return to the pool.
- The exhaustion failure paths stop being fatal and stop being inconsistent (F5, F6, F7 below).
- The new invariant — including the narrowed late-waiter contract — is written down in AGENTS.md.

### Non-Goals
- **No access control.** A token holder may still wait/consume a live owner's future; ownership is
  a cleanup net, not a gate. Capability enforcement is a separate concern if ever wanted.
- **No quotas and no occupancy gauge in this plan.** They are trigger-gated follow-ups (§11) built
  on the owner table this plan lays down; measure first, per house rule.
- **No detached launches here** except the kernel's own slice, kept as a follow-up decision (§11);
  `init`'s `service`/`hand_off` discards become group-owned and are swept when init exits, which
  already erases the boot-time leak without any new launch mode.
- **No waitset-treaty change.** `WaitsetCreate/Attach/Detach/Wait/Drop` behave exactly as today;
  only slot bookkeeping gains an owner.
- **No ABI change.** No syscall numbers, no wire formats; `system/` is untouched; usrlib and user
  space change zero.
- **No kill/terminate-other-task path.** The reclaim hook rides today's single termination site;
  a future kill path must respect §5 (blocked == live).

---

## 1. Verified baseline facts

### Code facts (cited against `adbcedf`)

- **F1 — One registry, one exit door.** `FutureRegistry` = `GenerationalArena<_, FUTURE_SLOTS>` +
  `waiters` map + `parents_of` rows (kernel/src/future/mod.rs:106-111). `FUTURE_SLOTS = 1024`
  (future/mod.rs:15). A slot is freed only through `arena.remove` under `consume`/`consume_inner`
  (future/mod.rs:178-196) — reached from the `WaitFuture` syscall (syscall.rs:66-88),
  kernel-internal `Kernel::wait` (kernel.rs:233), and `drop_waitset` (future/mod.rs:310-324).
  Nothing else ever reclaims.
- **F2 — Handles are unowned tokens.** `(index, generation)` packed (kernel/src/handle.rs); every
  borrow checks generation (collections/arena.rs:43-68). The kernel records nothing about who
  created or holds a slot.
- **F3 — Creation sites** (complete list; each must carry an owner after M2):
  `schedule_with_task_handle` TaskCompletionFuture (kernel.rs:193-196); `sleep` TimeFuture
  (kernel.rs:210-214); `WaitsetCreate` WaitsetFuture (syscall.rs:194); mailbox
  `pop_front_async` IpcMessageFuture ×3 (ipc/mailbox_manager.rs:86-99); `accept/receive_message_async`
  error-path futures (ipc/ipc_manager.rs:153,163); via `ForNotifyingFutures::register`
  (kernel_services.rs:37-39).
- **F4 — Completion futures are created in the parent's syscall.** `Spawn`/`LoadElf` call
  `schedule_with_task_handle` (kernel.rs:187) in the caller's context; the handle returns to the
  caller; the child only stamps it via `get_completion_future` at
  `terminate_with_status` (kernel.rs:297-301), which already **tolerates a missing future**
  (`if let Some`). Boot tasks (init, kstatsd) are scheduled with no caller task.
- **F5 — Exhaustion is fatal in two places.** `sleep` registers with `.expect("Failed to register
  sleep future")` (kernel.rs:214); every receive/accept registers with `.unwrap()`
  (mailbox_manager.rs:87,92,99; ipc_manager.rs:154,164). A full arena panics the kernel on the
  next sleep anywhere.
- **F6 — Exhaustion launches ghosts.** In `schedule_with_task_handle`, `add_task` succeeds, then a
  failed `register` skips `set_completion_future` **but `schedule_task` still runs** (kernel.rs:190-200)
  while the syscall reports failure — the task executes and nobody owns or can watch it.
- **F7 — WaitsetCreate is the only site that already degrades honestly** (error byte,
  syscall.rs:194-199).
- **F8 — Abandoned pending receives swallow messages.** `notify_waiters` pops a message, then pops
  a waiter and calls `complete_ipc_message` unconditionally (mailbox_manager.rs:103-119);
  on a dead/recycled handle the completion silently no-ops (kernel_services.rs:45-51) — the message
  is lost, the slot leaked.
- **F9 — The termination hook exists.** `terminate_with_status` already computes the group and
  gates `!group_has_live_members(group)` → `release_group_allocations` (kernel.rs:307-311);
  `group_has_live_members` is itself an arena sweep (task_manager.rs:79-82). `GroupId(usize)` is
  monotonic, never reused (task.rs:77).
- **F10 — Wake controller + IRQ discipline are already in place.** Registry mutations run under
  `FUTURE_SECTION` (future/mod.rs:17); wakeups go through `wake_controller` inside the registry
  (future/mod.rs:221-225). Reclaim composes with both; no new lock, no new section.

### Measured runtime facts **[O]** (2026-11-15, integration transcripts)

- **O1 — The clean-boot permanent ledger is ≈ 11 slots:** 8 `service`-step discards
  (init/src/main.rs:75, one per service: `rom:atad rom:blockd rom:vfsd rom:inputd rom:ps2d` +
  `/randomd /echo /pong`), 1 `hand_off` discard (init/src/main.rs:111), 2 kernel boot loads
  (init, kstatsd). `run` steps consume via `wait_future` (init/src/main.rs:101). Bounded per boot;
  the gauge (§11) should confirm the exact number.
- **O2 — A deliberate abandoner ships today:** `worker_outliving_main` spawns and never waits
  (apps/threads/src/main.rs:143). One slot leaked per threads run, permanent.
- **O3 — PR #169's shell reaper is the user-side complement:** background jobs are consumed exactly
  once at prompt boundaries (tests-integration/tests/shell_background.rs). It works because the
  shell stays alive to reap; it cannot help programs that spawn-and-exit (O2) or that die holding
  receive receipts. That is exactly the hole B closes.

---

## 2. Decisions (owner, before the milestones they gate)

- **D1 — Completion futures are owned by the SPAWNING (parent) group, not the subject's.** The
  parent received the handle (F4); zombie semantics: the parent must reap, parent death forfeits
  the report. Consequences verified below (§3, §5). Alternative (subject-owned) rejected: it frees
  the stamped slot at child termination, so a slow reaper loses `exited with code N` reports —
  it would break the live-shell contract O3 upholds.
- **D2 — Kernel-originated registrations get a `Kernel` pseudo-owner that is never swept.** The
  cost is O1's two boot slots. Accepted for purity of scope; §11 routes the erasure to the
  detached-launch follow-up. If the owner prefers zero residents now, the kernel slice of that
  follow-up folds into M2 with one extra commit.
- **D3 — The sweep does not reach into mailboxes.** Registry and mailbox stay decoupled; stale
  waiter entries self-evict on the generation check (F2), and F8's message loss is fixed
  independently in M0. A handle→mailbox reverse index is deferred (§11).
- **D4 — Ownership is not access control.** Consumption rights are unchanged while the owner
  lives. Stated so the diff is not read as capability work.
- **D5 — The late-waiter contract narrows to: "a waiter may consume while the issuing group
  lives; group teardown forfeits unreaped reports."** AGENTS.md's exit-status section currently
  states the unconditional form; M3 rewrites it (F9's teardown is the single, observable boundary).
- **D6 — No new error codes for the wake-abandoned path.** Woken waiters call `consume` on the
  removed slot and receive the existing not-found/already-consumed surface (kernel.rs:223-231);
  usrlib decodes it as it already decodes a double-wait. New bytes only if diagnosis says so.
- **D7 — Owner is threaded explicitly from the syscall edge.** `register(future, owner)` takes the
  owner; callers derive it once at the boundary (`current_task → get_group`), matching "the kernel
  validates once at the edge". No ambient current-task lookup inside the registry.

---

## 3. Design — ownership model

| Future | Owner | Normal exit | Abandon exit |
|---|---|---|---|
| `TimeFuture` | sleeping group | consumed inside the same `Sleep` syscall | only possible if sleep is interrupted (no such path today); net exists anyway |
| `IpcMessageFuture` | receiving group | `WaitFuture` with message out-ptr | swept at receiver-group death |
| `WaitsetFuture` | creating group | `WaitsetDrop` | swept at creator-group death; attached children stay with their own owners |
| `TaskCompletionFuture` | spawning parent group (D1); `Kernel` for boot loads | parent's `WaitFuture` (foreground wait, shell reaper) | swept at parent-group death; the child's later stamp targets a dead generation and no-ops |

Ownership by the **creator's group at registration time**, read at the syscall edge. Boot-time
registrations (no current task) are `Kernel`-owned (D2). `Spawn` siblings share the parent's group,
so sibling watches behave like shell background jobs. A task blocking on another group's future
remains legal and is woken with an honest error when the owner dies (§5).

## 4. Design — data structures and the reclaim path

```
FutureOwner: Group(GroupId) | Kernel | Free          // explicit enum; no magic-value collisions
FutureRegistry {
    arena, waiters, parents_of, wake_controller      // unchanged
    owners: Vec<FutureOwner>                         // parallel by slot index, sized FUTURE_SLOTS
}
```

- `register(future, owner)` stamps `owners[slot]` from the returned handle's index;
  `consume_inner` clears it beside the arena removal. A parallel `Vec` (a few KiB) keeps
  `collections::arena` group-agnostic.
- `reclaim_group(group)` under `FUTURE_SECTION`, called from the existing last-live-member gate
  in `terminate_with_status` (kernel.rs:307-311), beside `release_group_allocations`:
  1. scan `owners` for slots owned by `group` — O(1024) equality sweep, same shape as
     `release_group_allocations`' block walk and `group_has_live_members`' task walk (F9);
  2. wake parked waiters through `wake_controller` (they resume in `wait_future`, `consume` fails,
     the syscall answers the existing error — D6);
  3. `consume_inner` each slot (generation bump + `parents_of` purge) and clear the owner.
- Teardown ordering inside `terminate_with_status` is already correct for D1: the exiting task's own
  completion future is notified (kernel.rs:300) before the sweep, and is parent-owned, hence never
  in the swept set.

## 5. Safety argument

- **Keystone: generation.** `arena.remove` bumps the slot generation (arena.rs:65-66). After a
  sweep, every stale reference — mailbox waiter entry, child TaskRecord's completion handle,
  `parents_of` edge, token in dead statics — fails the generation check at every borrow site
  (`complete_ipc_message`, `notify_with_status`, `wait_future`/`consume`, `is_live`). A reclaimed
  slot may be recycled by any group with no possible corruption, and no dead handle can wake
  anything. This is the same mechanism that already makes consume-once honest.
- **Blocked == live.** `group_has_live_members` counts blocked tasks (F9): a group cannot die while
  one of its members is parked, so a sweep can never wake a same-group sleeper; only cross-group
  waiters exist at sweep time, and they receive the honest error. Any future kill path must
  preserve this reading.
- **Timer handles.** A `TimeFuture`'s only lifetime is its own syscall; the task cannot terminate
  while parked in it (no kill path). If a kill path ever lands, the TimerManager handle list must
  be purged like the mailbox lists — threat T3.
- **No self-reentry.** `reclaim_group` runs with preemption off inside termination, mutates only
  registry tables and the wake list, and never re-enters the task manager beyond what
  `wake_controller` already does from this context (notify does the same today).

## 6. Change surface

| File | Change | Milestone |
|---|---|---|
| kernel/src/ipc/mailbox_manager.rs | `notify_waiters`: liveness-check the waiter **before** popping the message; skip dead waiters (F8 fix) | M0 |
| kernel/src/kernel.rs | `schedule_with_task_handle`: register the completion future **before** `add_task` (it does not need the task handle), so exhaustion never ghosts a task (F6); `sleep`: failure without panic (F5) — returns an error register instead of `.expect`; reclaim call at the group gate | M1, M2, M3 |
| kernel/src/ipc/ipc_manager.rs, mailbox_manager.rs | receive/accept under registry pressure fail directly (return no handle) instead of `.unwrap()`-ing an error future (F5) | M1 |
| kernel/src/future/mod.rs | `FutureOwner`, `owners` table, `register(future, owner)`, clear-on-consume, `reclaim_group` + unit tests | M2, M3 |
| kernel/src/syscall.rs | pass edge-derived owner at `Spawn`/`LoadElf`/`Sleep`/receive/accept/`WaitsetCreate` | M2 |
| kernel/src/kernel_services.rs | `ForNotifyingFutures::register(future, owner)`; liveness check exposed for the mailbox fix | M0, M2 |
| AGENTS.md | future-registry ownership bullet; narrowed late-waiter contract (D5); exit-status section cross-reference | M3 |

## 7. Test plan

- **Registry units (M2/M3):** owner stamped on register, cleared on consume; `reclaim_group` frees
  only the owner's slots; parked cross-group waiter wakes and consumes to the honest error;
  `parents_of` rows purged by the sweep; a reclaimed-then-recycled slot is immune to stale-handle
  notify/stamp (generation wall); `Kernel`-owned slots survive every sweep.
- **Termination units (M3):** stamp-before-sweep ordering; a child's parent-owned completion future
  survives the child's own group teardown (D1 zombie preservation).
- **Integration (M3):** a leak-witness app: spawn children and exit without joining (generalizing
  O2), then prove registry reusability from another task afterwards; `threads` unchanged still
  passes; **PR #169's `shell_background` suite is the regression net for the preserved live-parent
  contract** — it must stay green without edits.
- **Build surfaces:** both arches at zero warnings, `cargo xtask check-isa x86_32`, host unit tests.

## 8. Milestones (each = one reviewable commit)

- **M0 — Stop eating messages.** Mailbox `notify_waiters` liveness-skip (F8). Independent of
  everything else; lands first on its own.
- **M1 — Exhaustion honesty.** Reorder `schedule_with_task_handle` (register before `add_task`) so
  a failed pairing cannot ghost a task (F6); `Sleep` and receive/accept return errors instead of
  panicking when the arena is full (F5). No ownership yet.
- **M2 — Ownership stamping.** `FutureOwner` + `register(future, owner)` + `owners` table, every
  call site carries the edge-derived owner (D7). Mechanically provable behavior change: nothing
  consumes the new field except a debug gauge assertion.
- **M3 — Reclaim.** `reclaim_group` + the `terminate_with_status` hook + wake semantics + full
  unit/integration coverage + AGENTS.md updates (D5). This is the milestone that closes the leak
  class.
- **Follow-ups (§11) stay out of this plan's milestones.**

## 9. Threat register

- **T1 — Sweeping a future something still legitimately references.** Every holder is enumerated in
  F3/F4/F8; the only cross-lifetime references are mailbox lists, `parents_of`, TaskRecord
  completion handles, and TimerManager handles — the first three are generation-checked at every
  use (F2), the fourth cannot outlive its own syscall today (§5). Any new holder must be
  generation-checked too.
- **T2 — O(1024) sweep inside the IRQ-guarded termination window.** Bounded and rare (group death);
  same shape as existing sweeps (F9). Revisit only if measurements show scheduling jitter.
- **T3 — A future kill path** could park-and-kill outside the blocked==live invariant (§5). Any
  such plan must purge timer/mailbox references in the same gate; recorded here so that plan
  inherits the obligation.
- **T4 — GroupId width.** `GroupId(usize)` monotonic (F9); the owner table stores the full value in
  an enum, no packing, so no wrap concerns; `Kernel` is a variant, never a magic id.
- **T5 — Live hoarding remains possible** (a running task abandoning slots starves the pool until it
  dies). Out of scope by D-gate; §11's quota is the planned answer, pressure is made visible by the
  gauge first.

## 10. Follow-ups (trigger-gated, not in this plan)

- **Detached launches (Option E):** kernel launches create no completion future (the stamp path
  already tolerates their absence, F4) → boot residents go to zero (O1); `init`'s `service` steps
  gain a fire-and-forget launch mode. Trigger: O1 confirmed by the gauge.
- **Occupancy gauge through kstatsd** (`ps`/`mem` view: per-group slot holdings, total) — the
  measurement layer for everything above; cheap once `owners` exists.
- **Per-group quota** at register (`count owners == g`), honest `QuotaExceeded` — the T5 answer.
- **Mailbox reverse index** for eager stale-waiter purge — only if a workload ever shows cost.
