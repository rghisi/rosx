# Composite Waitset (depth-1, Any-only) — Implementation Plan

> Planned 2026-10-22 at worktree `witty-orchid`, HEAD `da8d54b`.
> Inputs: Context Map `.opencode/artifacts/waitset-context-map.md` (its "Corrections / deviations" are binding),
> orchestrator verifications, and the locked decisions below. Every quoted string in this plan was
> re-verified verbatim against the tree at HEAD before writing.
> This plan writes documents only; the Coder implements. Repo law applies: `kernel/` is `no_std`,
> deps limited to `system` + `lazy_static`, **no comments ever** (assembly excepted), unit tests
> mandatory, `cargo build --workspace` is forbidden, commit subjects are bare imperative (no prefix).

---

## 0. Locked decisions (user-approved — CLOSED, not open questions)

- **LD1 Depth-1 only:** waitset children must be leaf futures; attaching a waitset as a child is rejected (`WaitsetNestingForbidden`). No rank/depth machinery this iteration.
- **LD2 `Any` (OR) only:** `WaitsetCreate` takes NO kind argument; no `WaitSetKind` enum on the wire. AND composition is a future iteration; do not scaffold it.
- **LD3** A waitset is a `FutureHandle` slot in the existing `FutureRegistry` arena (`kernel/src/future.rs`): no new handle newtype, no new registry, no new service/UseCase in `kernel_services.rs`.
- **LD4 Discovery is two-tier:** `WaitsetWait` returns `WaitEvent { child, tag }` written to a user out-pointer; payload retrieval reuses the EXISTING `WaitFuture(child, out_ptr)` arm unchanged (consume + `IpcMessageFuture` downcast).
- **LD5** Zero special-case logic for IRQ vs IPC in notify/cascade. Keyboard, mailbox, timer code paths are untouched. `HardwareInterrupt` gains no variants this change.
- **LD6** Wake fast-path O(1): reverse index is a flat `Vec` indexed by `handle.index()`; child vec and pending ring pre-sized at create; cascade allocation-free (take-and-restore row pattern); no `rg`, gates use POSIX `grep -E`.
- **LD7** Edges are generation-checked packed handles only — never references, no refcounts. Staleness is pruned, dangling unrepresentable.
- **LD8** End-to-end proof commit IS in scope (user-approved): new `apps/waitset_demo`, shell command, `xtask` USER_APPS extension, new scripted QEMU test `tests-integration/tests/waitset_or.rs` following the `ipc_ping.rs` sentinel discipline.

---

## 1. Verified baseline facts, and corrections to the delegation text

These were verified by reading the files; where a fact deviates from the delegation or Context Map,
**this section wins**.

- **F1 — There are FOUR ErrorCode-enumerating sites, not two.** The delegation (and orchestrator note)
  name `kernel/src/handle.rs:259` and `usrlib/src/error.rs:134`, both
  `const ALL_ERROR_CODES: [ErrorCode; 13] = [` (inside `#[cfg(test)]` mods). Verification found two more
  enumerating the same inventory: `system/src/error.rs:53`
  (`const ALL_ERROR_CODES: [ErrorCode; 13] = [`, used by `every_error_code_variant_roundtrips_through_the_register_codec`)
  and the inline list at `system/src/ipc.rs:56-70` inside `packed_handle_space_is_disjoint_from_error_space`.
  Precedent `plans/ipc-bulk-data-transfers-impl.md` BR-2 grew all four in one commit; this plan does the same (`; 13` → `; 17`).
- **F2 — `USER_APPS` is at `xtask/src/main.rs:5`**, not `:32-41` (that range is `build_apps()`, which consumes the list).
  Exact current line:
  `const USER_APPS: [&str; 11] = ["hello_elf", "random_gen_server", "snake", "tetris", "conway", "bulk_echo", "bulk_client", "ping_server", "ping_client", "vfs_server", "vfs_client"];`
  (`pack_rom()` at `xtask/src/main.rs:75-98` iterates the same list, so the ROM grows automatically.)
- **F3 — BINDING demo-routing correction:** the delegation's demo flow names `ipc_receive_message` as the child
  producer. `IpcManager::send_message` (`kernel/src/ipc/ipc_manager.rs:126-142`) routes by caller identity:
  `if caller == conn.client_task { conn.server_mailbox } else { conn.client_mailbox }`, while
  `receive_message_async` (`:155-163`) pops the **client** mailbox. A task that binds a service and connects to
  it sends *only* to the server mailbox, so a literal `ipc_receive_message` child would never complete and
  `waitset_wait` would hang. **The demo's child per round is the future from `Syscall::ipc_accept_message(binding)`**
  (`accept_message_async` → `pop_front_async(server_mailbox)`, `ipc_manager.rs:144-153`), matching the proven
  manager-level exchange `client_send_accept_delivers_envelope` (`ipc_manager.rs:243-255`) built by
  `bind_and_connect` (`kernel/src/ipc/ipc_manager.rs:230-234`).
- **F4 — Apps cannot fabricate user handles:** `UserFutureHandle::from_token` and `packed()` are `pub(crate)`
  (`usrlib/src/handle.rs:13-23`). Precedents: `usrlib::arch::raw_syscall` is `pub`
  (`usrlib/src/arch/x86_64.rs:1`, identical shape on x86_32 per map §11), and `apps/ping_client/src/main.rs:26-28`
  recovers a packed word via `Display`:
  `u32::from_str_radix(&format!("{}", handle), 16).expect("canonical data token")`.
  The demo's bogus-child negative probe uses `raw_syscall` with a `Display`-derived set token (F4 precedent) and
  a literal well-formed-but-absent child word `0x7FFF_FFFF` (a token per `is_token`, see `usrlib/src/handle.rs:187`).
- **F5 — Integration tests live in `tests-integration/tests/`** (7 files: `app_lifecycle`, `boot_and_shell`,
  `elf_arguments`, `ipc_bulk`, `ipc_ping`, `ipc_random`, `ipc_vfs`), verified by directory listing; `src/` holds the
  harness (`qemu.rs`, `disk_image.rs`, `kernel_build.rs`, `monitor.rs`, `output.rs`, `sendkey.rs`, `lib.rs`).
  `tests-integration/src/kernel_build.rs` builds **only the kernel** (`--manifest-path arch/x86_64/Cargo.toml --target arch/x86_64/rosx.json`
  plus the `-Z` triple); it never builds user ELFs. Since the shell embeds
  `include_bytes!("../../../target/rosx-user/release/waitset_demo")` (Step 15) and `arch/x86_64/src/main.rs:70` embeds
  `rom.img`, **`cargo xtask build` must run before any integration gate** so those artifacts exist.
- **F6 — Observed, do not fix:** the dead-handle panic hazard at `kernel/src/syscall.rs:53`
  (`let future = kernel().wait_future(handle).unwrap();`): `wait_future` consumes and `consume` fails `NotFound`
  for an unknown/recycled handle, so `unwrap()` aborts. Pre-existing; out of scope; this plan only records it.
  The WaitFuture waitset-guard (Step 12) must not mask it and must not introduce any new unwrap.
- **F7 — `cargo xtask test --skip-integration`** runs `build_apps()` + `pack_rom()` first, then
  `cargo test --workspace --exclude tests-integration -- --test-threads=1` (`xtask/src/main.rs:100-113`).
  That is why the shell's host-side `include_bytes!` (shell is a default-member, and `target_arch = "x86_64"`
  holds on the host) resolves after Step 16 adds `waitset_demo` to `USER_APPS`.
- **F8 — x86_32:** no CI job exists (map correction 2). The new syscall arms compile for x86_32 transitively
  because `handle_syscall` (`kernel/src/syscall.rs:14`) is shared and entered from `arch/x86_32/src/cpu.rs:108`;
  usrlib arch glue is unchanged. No x86_32 gate is bound (recipe is doc-only and `unverified`).
- **F9 — `system/src/lib.rs` export style** is bare `pub mod <name>;` lines
  (`pub mod syscall_numbers; pub mod ipc; pub mod error; pub mod rom;`). `pub mod waitset;` matches.
- **F10 — Verified anchor quotes used below** (all exact): `    RomRead = 27,` (`system/src/syscall_numbers.rs:24`);
  `            27 => Ok(Self::RomRead),` (`:52`); `IpcDataNotReadable = 0x010C,` then `    Unknown = 0x7FFF_FFFF,`
  (`system/src/error.rs:21-22`); `            0x010C => ErrorCode::IpcDataNotReadable,` then `            _ => ErrorCode::Unknown,`
  (`:43-44` — `from_code` fallback confirmed); `Ok(SyscallNum::WaitFuture)` arm `:50-62` with the out-pointer precedent
  `unsafe { core::ptr::write(out_ptr as *mut Message, message) }` (`:56`); `Ok(SyscallNum::IpcDisconnect)` idempotent
  precedent `:94-99`; `notify` `:160-163`; arena generic inline `1024` at `future.rs:97`;
  `pub fn wait_future(&mut self, handle: FutureHandle) -> Result<Box<dyn Future + Send + Sync>, Error>` (`kernel.rs:224`);
  `Message` is `#[repr(C)]` with `conn: u32` (`system/src/ipc.rs:4-9`); `pack()` masks generation to
  `GENERATION_MASK = 0x7FFF` so `pack() < 2^31` (`kernel/src/handle.rs:5,22-29`) — the `pack() as u32` encode is lossless.
- **F11 — Delivery chain the cascade rides on (verified):** `MailboxManager::push_back` (`mailbox_manager.rs:58-70`)
  calls `notify_waiters` (`:93-109`), which does `notifier.complete_ipc_message(fh, msg)` then `notifier.notify(fh)`;
  `FutureRegistryNotifierUseCase::notify` (`kernel_services.rs:32-52`) reaches `FutureRegistry::notify`. All IRQ-driven
  notification is deferred to the scheduler-thread drain (map §9), so the cascade inherits that safety.
- **F12/F13 — Orchestrator amendments (post-authoring, re-verified at HEAD `da8d54b`):** (a) acceptance gate #6's
  pure-append pattern was bound as `grep -E "^-[^-]"` — the original bare `^-` also matches the diff's `--- a/`
  header line and could never produce no output; (b) Step 14's members anchor was bound to `after
  "apps/test_suite",` (`Cargo.toml:22`, the last apps member) — the original named `"apps/vfs_client"`, which is not
  the list tail. All other anchors spot-verified green: the four `; 13]`-class inventory sites (`system/src/error.rs:53`,
  `kernel/src/handle.rs:259`, `usrlib/src/error.rs:134`, inline list at `system/src/ipc.rs:56`), `USER_APPS` verbatim
  at `xtask/src/main.rs:5`, caller-identity routing per F3, `from_token`/`packed` `pub(crate)` and the
  `Display`-token precedent (`apps/ping_client/src/main.rs:26-28`), the shell `PING_ELF`/insert/`fn ping` sites,
  `README.md:181` tree line, `AGENTS.md:42` and `AGENTS.md:149`, `pub mod rom;` as last line of `system/src/lib.rs`,
  and the `apps/ping_client/Cargo.toml` shape (mirror it including the `authors`/`version` lines).

Command verification status used throughout (standing rule: a gate marked `unverified` is NOT binding until the
orchestrator's baseline pass proves it on a green tree):

| Command | Status | Source |
| --- | --- | --- |
| `cargo xtask test --skip-integration` | CI-proven form (`.github/workflows/ci.yml:31`) | map §12 |
| `cargo xtask build` | CI-proven form (`ci.yml:51`) | map §12 |
| `cargo test -p system` | `unverified` (not a CI command; `system` is a default-member host crate) | `Cargo.toml:27` |
| `cargo test -p tests-integration --test waitset_or` | `unverified` — scripted QEMU gate (harness exists, map §13); never "manual" | map §13 |
| `cargo test -p tests-integration --test boot_and_shell` | `unverified` — scripted QEMU gate | map §13 |
| `cargo test -p tests-integration --test ipc_ping` | `unverified` — scripted QEMU gate | map §13 |
| POSIX `grep -E` gates | provable from file text | this plan |

All QEMU integration tests are **excluded from CI** (`cargo xtask test --skip-integration` is the CI test command;
`--exclude tests-integration` at `xtask/src/main.rs:106`); their gates above are therefore **local/verifier-tier**,
executed by the orchestrator/verifier on a host with `qemu-system-x86_64`, not by CI.

---

## 2. Wire contract (frozen exactly; append-only treaty)

`system/src/syscall_numbers.rs` — append after `    RomRead = 27,`:
`WaitsetCreate = 28`, `WaitsetAttach = 29`, `WaitsetDetach = 30`, `WaitsetWait = 31`, `WaitsetDrop = 32`.
Mirror arms after `            27 => Ok(Self::RomRead),` and before `            _ => Err(()),`.
Existing numbers/values are frozen: gaps 13, 14, 16–19, 21 stay untouched.

`system/src/error.rs` — new family inserted after `IpcDataNotReadable = 0x010C,` and before `Unknown = 0x7FFF_FFFF,`:
`WaitsetFull = 0x0201`, `WaitsetChildInvalid = 0x0202`, `WaitsetNestingForbidden = 0x0203`, `WaitsetNotFound = 0x0204`;
matching `from_code` arms inserted after `0x010C => ErrorCode::IpcDataNotReadable,`. Existing `0x0101..0x010C` +
`Unknown` frozen.

`system/src/waitset.rs` (new) + `pub mod waitset;` in `system/src/lib.rs` (F9 style):
`#[repr(C)] #[derive(Copy, Clone, Debug, PartialEq, Eq)] pub struct WaitEvent { pub child: u32, pub tag: u32 }`
— fixed-width treaty like `Message.conn: u32`. Treaty tests (ipc.rs style): `size_of::<WaitEvent>() == 8`,
`offset_of!(WaitEvent, child) == 0`, `offset_of!(WaitEvent, tag) == 4`.
Kernel encodes `FutureHandle::pack() as u32` (lossless per F10); usrlib decodes via
`UserFutureHandle::from_token(event.child as usize)` inside the wrapper.

Syscall semantics (`arg3`-slot budget respected; `handle_syscall` takes exactly 3 argument slots):

| Call | Success | Failure |
| --- | --- | --- |
| `WaitsetCreate(arg unused)` | `set.pack()` | `WaitsetFull.to_reg()` (arena exhausted) |
| `WaitsetAttach(set, child, tag)` | `0` | validation matrix (§3.4); malformed `set` raw → `WaitsetNotFound`, malformed `child` raw → `WaitsetChildInvalid` |
| `WaitsetDetach(set, child)` | `0` | `WaitsetNotFound` (set gone/not-a-waitset) \| `WaitsetChildInvalid` (edge absent / malformed child raw) |
| `WaitsetWait(set, out_ptr)` | `0`; writes `WaitEvent` via `core::ptr::write` **only when `out_ptr != 0`** (precedent `syscall.rs:56`) | `WaitsetNotFound` (set gone); malformed set raw → `WaitsetNotFound` |
| `WaitsetDrop(set)` | always `0`, idempotent (precedent `IpcDisconnect`, `syscall.rs:94-99`; malformed raw ignored) | none |

Both/all `ALL_ERROR_CODES: [ErrorCode; 13]` arrays (three sites per F1) become `; 17` with the four new variants
inserted in code order before `Unknown`, and the inline list at `system/src/ipc.rs:56-70` gains the same four;
usrlib totality tests extended accordingly. New `WaitsetError` mirror enum in `usrlib/src/error.rs` following the
exact precedent shape (specific arms + catch-all → `NotFound`).

---

## 3. Kernel design contracts (contracts, not code)

### 3.1 Module reorg (zero behavior change)
`kernel/src/future.rs` → `kernel/src/future/mod.rs` — verbatim content move (its `#[cfg(test)] mod tests`
travels with it; `crate::future::...` paths across the tree keep resolving; `lib.rs:12` `pub mod future;` unchanged)
plus `pub(crate) mod waitset;` inside `mod.rs` and the new file `kernel/src/future/waitset.rs`.
Introduce `pub(crate) const FUTURE_SLOTS: usize = 1024;` in `future/mod.rs`, used BOTH by the arena generic
(today inline `1024` at `future.rs:97`) and the reverse-index sizing.

### 3.2 `WaitsetFuture` (`pub(crate)`, in `future/waitset.rs`, implements the `Future` trait)
```
pub(crate) const WAITSET_CHILD_CAPACITY: usize = 16;
pub(crate) const WAITSET_PENDING_CAPACITY: usize = 16;
pub(crate) struct WaitsetChild { child: FutureHandle, tag: u32 }        // Copy
pub(crate) struct WaitsetFuture {
    children: Vec<WaitsetChild>,            // Vec::with_capacity(WAITSET_CHILD_CAPACITY) at new()
    pending: VecDeque<WaitEvent>,           // VecDeque::with_capacity(WAITSET_PENDING_CAPACITY) at new()
}
```
`is_completed()` = `!pending.is_empty()`; `complete()` stays the inherited default no-op;
`as_any`/`as_any_mut`/`into_any` follow the `TimeFuture` pattern (`future.rs:44-54`).
Methods: `new()`; `record(&mut self, event: WaitEvent) -> bool` (push_back; returns true iff `pending` was empty
before the push — "parent became signalled"); `take_pending(&mut self) -> Option<WaitEvent>` (`pop_front`);
`remove_child(&mut self, child: FutureHandle) -> bool` (drop the edge **and** purge `pending` events whose
`event.child == child.pack() as u32`; true iff an edge was removed); `has_child(FutureHandle) -> bool`;
`child_count(&self) -> usize`. `pending` stays bounded (≤1 event per attached edge; purges on detach/consume);
`VecDeque` growth beyond capacity is defensive only — never drop, never panic.

### 3.3 `FutureRegistry` additions (`future/mod.rs`)
- Field `parents_of: Vec<Vec<ParentLink>>`, length `FUTURE_SLOTS`, built in **both** `new` and
  `new_with_wake_controller` (element type `#[derive(Copy, Clone)] pub(crate) struct ParentLink { parent: FutureHandle, tag: u32 }`).
  1024 empty `Vec`s allocate nothing.
- `pub(crate) enum AttachError { ParentNotFound, ChildInvalid, NestingForbidden, Full }` (kernel-domain enum;
  wire mapping lives only in the syscall arm, per the ownership principles).
- `pub(crate) enum WaitsetTake { Event(WaitEvent), Empty, Gone }` — "Gone" = set absent **or** present-but-not-a-waitset.
- `pub(crate) fn attach(&mut self, set: FutureHandle, child: FutureHandle, tag: u32) -> Result<(), AttachError>`
- `pub(crate) fn detach(&mut self, set: FutureHandle, child: FutureHandle) -> Result<(), AttachError>`
- `pub(crate) fn take_pending(&mut self, set: FutureHandle) -> WaitsetTake`
- `pub(crate) fn drop_waitset(&mut self, set: FutureHandle)`
- private `fn is_live(&mut self, handle: FutureHandle) -> bool` — `arena.borrow(handle.into()).is_ok()` (generation-checked).

`attach` behavior — validation matrix (bind as a table; order matters, checked top-down):

| Condition | Result |
| --- | --- |
| parent absent | `AttachError::ParentNotFound` → `WaitsetNotFound` |
| parent present but not a waitset | `AttachError::ParentNotFound` → `WaitsetNotFound` |
| child absent (arena borrow `NotFound`, includes wrong generation) | `AttachError::ChildInvalid` → `WaitsetChildInvalid` |
| child is a waitset | `AttachError::NestingForbidden` → `WaitsetNestingForbidden` (subsumes `child == parent`) |
| duplicate child already in set | `AttachError::ChildInvalid` → `WaitsetChildInvalid` |
| children at cap (`child_count() == WAITSET_CHILD_CAPACITY`) | `AttachError::Full` → `WaitsetFull` |
| **child already completed** | attach succeeds, **latches the event immediately and calls `notify(set)`** |
| otherwise | push edge; push `ParentLink` into `parents_of[child.index() as usize]` |

The latch rule exists because a message-already-queued `pop_front_async` future registers
completed-without-notify (map §8, `mailbox_manager.rs:74-77`); without the latch `WaitsetWait` would hang.
Check child completion via the arena borrow's `Future::is_completed()`.

`detach`: set absent/not-a-waitset → `ParentNotFound`; edge absent → `ChildInvalid`. On success:
`remove_child` on the parent **and** remove the matching `ParentLink` (by `link.parent == set`) from
`parents_of[child.index() as usize]` — detach that forgot the reverse row would later record events the set
detached from (a live-parent borrow would succeed, so the cascade could not self-heal it).

`take_pending`: borrow set → absent or downcast-miss → `Gone`. Else loop `pop_front`; before returning an event,
validate `is_live(child-from-event-token)` via `FutureHandle::from_packed(event.child as usize)` (a token written
by the kernel always parses; parse-failure or dead-generation = stale event — drop it and continue); queue drained → `Empty`.
This staleness gate (LD7) is the consumer-side backstop for notify-after-recycled-generation (map correction 6c).

`consume` hardening: today `pub fn consume(...) -> Result<..., Error>` is `self.arena.remove(handle.into())`.
Hardened contract: run `arena.remove` first; **on successful removal only**, take (`core::mem::take`) the row at
`parents_of[handle.index() as usize]`, and for each link whose parent borrows live and downcasts to `WaitsetFuture`,
call `remove_child(handle)` on it; do **not** restore the row (slot-recycle safety: a future occupant of this index
starts with an empty reverse row). On `arena.remove` error, return the error unchanged, row untouched.

`notify` (`future.rs:160-163`) keeps its two existing lines untouched (`waiters.remove` +
`wake_controller.wake_tasks`), then cascades:
1. `core::mem::take` the row at `handle.index()` (allocation-free; `Vec` is the same allocation on restore).
2. If the row is non-empty **and** `is_live(handle)`: for each link — borrow the parent, downcast to `WaitsetFuture`,
   `record(WaitEvent { child: handle.pack() as u32, tag: link.tag })`; a link survives iff the parent borrow succeeds
   **and** the parent is a `WaitsetFuture` (failure = prune by not restoring); collect parents for which `record`
   returned true.
3. Restore the surviving row to `parents_of[index]` **unconditionally on every path** (including the
   row-non-empty-but-child-dead case: that row belongs to a different, live generation of the same slot —
   restore it untouched; the dead child handle's links are not this row's provenance and the generation gate forbids recording).
4. Only after restore, recurse `notify(parent)` for each satisfied parent collected in step 2. LD1 makes this
   max 2 levels (a waitset is never a child, so a waitset's own row is always empty → recursion terminates one level in).
5. Borrow discipline: every arena borrow must be dropped before the next registry operation (decide-inside /
   act-outside the borrow). `wake_controller.wake_tasks` re-enters a *different* `KernelCell` (`scheduler`),
   which map §6 confirms is the established pattern.

`drop_waitset(set)`: for each attached child, prune this set's `ParentLink` from `parents_of[child.index()]`
(drop edges whose `parent == set`, handle equality including generation); then `notify(set)` to release any
blocked waiters (they observe `Gone` in their loop → error); then `arena.remove(set)`, result ignored.
`WaitsetDrop` on an absent set is a no-op that still returns 0.

### 3.4 `Kernel::wait_waitset` (kernel.rs)
```
pub fn wait_waitset(&mut self, handle: FutureHandle) -> Result<WaitEvent, Error>   // Error = crate::collections::arena::Error
```
Loop — `take_pending(handle)`: `Event(ev)` → `Ok(ev)`; `Empty` → `block_current_task()` +
`register_waiter(handle, current_task)` + `switch_to_scheduler()` + retry (this flow mirrors
`wait_future` at `kernel.rs:224-232`; the retry top-of-loop is the spurious-wake guard for broadcast wake
and for a task that also waited directly on a child); `Gone` → `Err(Error::NotFound)`.
**NEVER consumes the waitset** (multiple rounds per set). This becomes the *second* production caller of
`register_waiter` (map correction 4's "one production caller" fact updates accordingly — no doc states that count).

### 3.5 `WaitFuture` guard (`syscall.rs:50-62` arm)
In the `WaitFuture` arm, BEFORE waiting/consuming, probe the registry: if the target borrows live and downcasts
to `WaitsetFuture`, return `ErrorCode::Unknown.to_reg()` **without consuming**. The probe must be an expression
producing a bool with no borrow surviving into the subsequent flow (no nested `KernelCell` borrows; the existing
`wait_future` path then proceeds unchanged). If the probe borrow misses (absent handle), the existing path
proceeds unchanged — including the pre-existing dead-handle `unwrap()` panic hazard at `syscall.rs:53`
(F6: recorded as an observed out-of-scope item; do **not** fix it silently here).

### 3.6 Untouched (hard boundary)
`keyboard.rs`, `mailbox_manager.rs`, `message_future.rs`, `kernel_services.rs` wiring,
`wait_future`/`wait::<T>`/`replace` (the dead API from map correction 3 must keep compiling), `HardwareInterrupt`
(`kernel/src/messages.rs:3-6`), `arch/` glue, `NoopWakeController`/`DEFAULT_WAKE_CONTROLLER`,
`wire_scheduler_for_tests` (`scheduler/mod.rs:49-64`) must stay alive and green.

### 3.7 Generation/recycling safety (derived, LD7; binding)
- `consume` is the only path that clears a reverse row; every slot removal goes through `consume` or `drop_waitset`,
  so a non-empty row implies the current occupant attached it — but the cascade still gates on `is_live(handle)`
  (step 2 above), which kills stale-handle notifies (e.g. mailbox stale waiters, map correction 6a) from recording
  spurious events into a recycled slot's parents.
- `take_pending`'s event-child liveness gate (F3.3) is the second line of defense for events that became stale
  after recording (two sets sharing one child where the other path consumed it).
- Consequence: the waitset path never hands the user a dead child token, which would otherwise detonate the F6
  `unwrap()` in the `WaitFuture` arm.

---

## 4. Sentinel cross-binding (Steps 14 and 17 MUST agree byte-for-byte)

The demo prints exactly these lines (per round tags 11/22/33); a mismatch in either file fails the C4 gate:

```
waitset round: 1 tag: 11 ok
waitset round: 2 tag: 22 ok
waitset round: 3 tag: 33 ok
waitset negative: nesting rejected ok
waitset negative: child rejected ok
waitset negative: waitfuture guarded ok
waitset negative: drop idempotent ok
waitset complete: 3 rounds ok
```

Any unexpected outcome prints the same line shapes with the final word `fail` instead of `ok`
(`verdict_word` precedent, `apps/ping_client/src/main.rs:30-32`). No ok-sentinel contains the substrings
`fail` or `Panic`. The test's constants (Step 17):
`ROUND_PREFIX = "waitset round: "`, `NEGATIVE_PREFIX = "waitset negative: "`,
`LAST_LINE = "waitset complete: 3 rounds ok"`, `FORBIDDEN_STRINGS: [&str; 2] = ["fail", "Panic"];`.

---

## 5. Steps (execution order; one commit per C-group, each green at its own boundary)

### Commit C1 — treaty only (system additions + ALL arrays + mirror + totality tests)

**Step 1.** `system/src/syscall_numbers.rs`: append the five variants and five `TryFrom` arms per §2
(insert after the two quoted anchor lines, F10). Why: append-only treaty numbers 28–32; nothing consumes them yet.
Verify: `cargo test -p system` — `unverified` → route to baseline pass; fallback check `cargo xtask test --skip-integration` (CI-proven form).

**Step 2.** `system/src/error.rs`: (a) insert the four variants after `    IpcDataNotReadable = 0x010C,` before
`    Unknown = 0x7FFF_FFFF,`; (b) insert the four `from_code` arms after `            0x010C => ErrorCode::IpcDataNotReadable,`
before `            _ => ErrorCode::Unknown,`; (c) grow the test-mod `ALL_ERROR_CODES` from `[ErrorCode; 13]` to
`[ErrorCode; 17]` with the four new entries in code order before `ErrorCode::Unknown` (the existing roundtrip test
then proves codec for free — precedent ipc-bulk BR-2).

**Step 3.** `system/src/ipc.rs`: in `packed_handle_space_is_disjoint_from_error_space`, add the four new variants
to the inline iteration list (`:56-70`) before `ErrorCode::Unknown` (F1 site four).

**Step 4.** Create `system/src/waitset.rs` per §2 (`WaitEvent` + treaty size/offset tests) and add `pub mod waitset;`
to `system/src/lib.rs` after `pub mod rom;`.

**Step 5.** `kernel/src/handle.rs` test mod: grow `ALL_ERROR_CODES` (`:259`) to `[ErrorCode; 17]` with the four
entries in code order before `Unknown`. The disjointness test `packed_handle_space_is_disjoint_from_error_space`
(`:380`) then proves the new codes stay out of the packed-token space.

**Step 6.** `usrlib/src/error.rs`: (a) grow `ALL_ERROR_CODES` (`:134`) to `[ErrorCode; 17]` (code order, before `Unknown`);
(b) add `pub enum WaitsetError { Full, ChildInvalid, NestingForbidden, NotFound }` with
`#[derive(Debug, Copy, Clone, PartialEq, Eq)]`, `From<ErrorCode>` (specific arms for the four new codes... three
specific + `WaitsetNotFound` folded into the catch-all `_ => Self::NotFound`, matching the `DataError` precedent
shape `error.rs:108-117`), and `Display` arms ("Waitset capacity exhausted", "Waitset child invalid",
"Waitset nesting forbidden", "Waitset not found"); (c) extend `every_error_code_maps_through_every_mirror_totally`
with a `waitset` mapping + `expected_waitset` match arm (`WaitsetFull → Full`, `WaitsetChildInvalid → ChildInvalid`,
`WaitsetNestingForbidden → NestingForbidden`, `_ → NotFound`) and assert it in the loop; (d) add
`assert_eq!(WaitsetError::from(ErrorCode::Unknown), WaitsetError::NotFound);` to
`unknown_code_falls_back_to_catch_all_variant_of_every_mirror`. Why: mirror enum precedent lives in `error.rs`,
not `ipc.rs` (map correction 1).

**C1 gate:** `cargo test -p system` (unverified) + `cargo xtask test --skip-integration` (CI-proven form).
Blast radius: pure additions; no existing behavior touched. Commit subject example: `Add waitset wire treaty and error codes`.

### Commit C2 — kernel waitset domain + registry cascade + module reorg + unit tests (feature inert: no syscall can create a waitset; every existing `notify` hits the empty-row path)

**Step 7.** Reorg per §3.1: move `kernel/src/future.rs` byte-identically to `kernel/src/future/mod.rs`
(git mv or read+write+delete by the Coder), add `pub(crate) mod waitset;` and
`pub(crate) const FUTURE_SLOTS: usize = 1024;`, and change the arena field type to
`GenerationalArena<Box<dyn Future + Send + Sync>, FUTURE_SLOTS>`. Zero consumer-visible change: every
`crate::future::...` import (e.g. `ipc_manager.rs:5`, `syscall.rs`, `kernel.rs`) still resolves.
Gate: `cargo xtask test --skip-integration` — future-side tests unchanged and passing (zero failures).

**Step 8.** Create `kernel/src/future/waitset.rs` with §3.2 types (`WaitsetChild`, `WaitsetFuture` + `Future` impl),
`ParentLink`, `AttachError`, `WaitsetTake` (§3.3), the two capacity consts, and `use system::waitset::WaitEvent;`
(the only needed import family: `alloc::vec::Vec`, `alloc::collections::VecDeque`, `crate::handle::FutureHandle`,
`core::any::Any`, `super::Future`). No comments anywhere (AGENTS law).
Vocabulary guard (LD1/LD2): never use the substrings `WaitSetKind`, `MAX_RANK`, or `rank` in this file —
the final acceptance grep forbids them.

**Step 9.** `future/mod.rs` — `FutureRegistry` per §3.3 + §3.7: add `parents_of` field initialized to
`FUTURE_SLOTS` empty rows in both constructors (shared private helper ok); implement `attach` (validation matrix
order, latch-with-`notify(set)`), `detach` (both-side removal), `take_pending` (liveness gate), `drop_waitset`
(prune child rows → `notify(set)` → `arena.remove`, ignored), `is_live`; harden `consume`; extend `notify` with
the cascade (existing two lines verbatim, then §3.3 steps 1–5, restore-row-unconditionally, recursion-after-restore,
allocation-free take-and-restore). Why-order-matters: `attach`'s latch consults `is_completed` while holding the
child borrow, then records, then calls `self.notify(set)` after that borrow ends.

**Step 10.** Comprehensive unit tests, in `#[cfg(test)] mod tests` blocks in `future/waitset.rs` (and cascade
tests in `future/mod.rs`'s moved test mod, reusing its `setup()` + `wire_scheduler_for_tests` pattern —
`wire_scheduler_for_tests` must stay alive):
1. fresh waitset `is_completed` false; `take_pending` → `Empty`.
2. attach validation matrix — one test per row (6 rows), incl. `child == parent` → `NestingForbidden` and
   duplicate → `ChildInvalid`; assert parents_of untouched on every rejection.
3. attach latch: pre-completed child (a `DummyCompletedFuture` in the test mod) → event pending immediately,
   `is_completed` true, `take_pending` returns it with the attach-time tag.
4. `record` FIFO + became-signalled semantics (first record true, second false).
5. cascade wake: register `WaitsetFuture` parent + leaf child; attach; `register_waiter(parent, blocked_task)`;
   `notify(child)` → event recorded **and** task Blocked→Ready (pattern of `notify_moves_waiters_to_ready`,
   with services()/wired scheduler).
6. fan-in: one child, two sets → both record; both drain independently.
7. cascade prune: parent consumed first, then `notify(child)` → no panic, parent link dropped, task untouched.
8. `consume` hardening: attach, record nothing, `consume(child)` → parent edge gone and `parents_of` row empty;
   re-register into the recycled slot and attach fresh → old links cannot resurrect.
9. stale-notify generation guard: attach child, consume, fill the recycled slot, notify with the stale handle →
   no event recorded (is_live gate; row of the new occupant restored untouched).
10. `take_pending` liveness gate: record an event, consume the child (direct `arena` path via `consume`), then
    `take_pending` → `Empty` (stale event dropped). (Note interplay with step-8 consume hardening: construct via
    two sets — consume through one set's hardening leaves the other set's event stale; assert second `take_pending` → `Empty`.)
11. `detach` purges both sides: detach → no later `notify(child)` records; re-attach of same child succeeds.
12. `drop_waitset`: task blocked via `register_waiter(set, task)`, `waiters` primed; `drop_waitset` → task woken
    (Blocked→Ready) and `take_pending` → `Gone`; double `drop_waitset` is a no-op.
13. `notify` on a handle with empty row is a no-op (already covered by the two moved tests passing unchanged — cite, don't duplicate).
14. `take_pending` on a non-waitset handle → `Gone`; on absent handle → `Gone`.

**C2 gate:** `cargo xtask test --skip-integration` (zero failing tests; feature inert — no syscall can create a
waitset yet, every existing `notify` call site sees an empty row, `consume`'s hardening short-circuits on an
always-empty row) + `cargo xtask build` (CI-proven forms). Commit subject example: `Add depth-one waitset future with registry wake cascade`.

### Commit C3 — `wait_waitset` + 5 syscall arms + `WaitFuture` guard + usrlib wrappers

**Step 11.** `kernel/src/kernel.rs`: add `pub fn wait_waitset(&mut self, handle: FutureHandle) -> Result<WaitEvent, Error>`
per §3.4 (loop contract; imports: `system::waitset::WaitEvent`, `crate::future::waitset::WaitsetTake`).
Why before the arms: Step 12's arm consumes it.

**Step 12.** `kernel/src/syscall.rs`: add the five arms per §2 table (match arms on `SyscallNum::WaitsetCreate` …
`::WaitsetDrop`, before `Err(_) => 0`). Contracts:
- Create: `services().future_registry.borrow_mut().register(Box::new(WaitsetFuture::new()))` →
  `Some(h) => h.pack()`, `None => ErrorCode::WaitsetFull.to_reg()`.
- Attach: parse set/child with `FutureHandle::from_packed` (malformed → per §2 table); `tag = arg3 as u32`
  (opaque kernel-side, never interpreted); `attach` → `Ok(0)` / `Err(e)` mapped:
  `ParentNotFound → WaitsetNotFound`, `ChildInvalid → WaitsetChildInvalid`, `NestingForbidden → WaitsetNestingForbidden`,
  `Full → WaitsetFull`.
- Detach: same parsing; `Ok(0)` / `ParentNotFound → WaitsetNotFound` / `ChildInvalid → WaitsetChildInvalid`.
- Wait: `kernel().wait_waitset(handle)`: `Ok(event)` → write only if `out_ptr != 0` via
  `core::ptr::write(out_ptr as *mut WaitEvent, event)` (precedent `syscall.rs:56`), return `0`;
  `Err(_) => ErrorCode::WaitsetNotFound.to_reg()`.
- Drop: mirror the `IpcDisconnect` shape (`syscall.rs:94-99`): `if let Some(set) = FutureHandle::from_packed(arg1) { ...drop_waitset(set); } 0usize`.
- `WaitFuture` guard per §3.5 (probe-first, `ErrorCode::Unknown.to_reg()`, no consume, no borrow surviving;
  F6 hazard stays as-is and gets a line in the commit body — the repo's commit bodies carry observations; the
  *subject* stays bare imperative).
Note: `handle_syscall` is `#[cfg(not(test))]` — this step is compiled by the cross-target build, not the unit gate.

**Step 13.** `usrlib/src/syscall.rs`: add five `Syscall` methods (decode precedent `:50-54`):
```
pub fn waitset_create() -> Result<UserFutureHandle, WaitsetError>   // from_token(raw), else Err(WaitsetError::from(ErrorCode::from_code(raw)))
pub fn waitset_attach(set: UserFutureHandle, child: UserFutureHandle, tag: u32) -> Result<(), WaitsetError>   // is_token(raw) branch
pub fn waitset_detach(set: UserFutureHandle, child: UserFutureHandle) -> Result<(), WaitsetError>
pub fn waitset_wait(set: UserFutureHandle) -> Result<(UserFutureHandle, u32), WaitsetError>
pub fn waitset_drop(set: UserFutureHandle)                          // result ignored; kernel always 0
```
`waitset_wait` contract: stack `WaitEvent` zero-initialized, pass `&mut event as *mut WaitEvent as usize` as arg2;
on `is_token(raw)` decode `UserFutureHandle::from_token(event.child as usize)` — `None` maps to
`Err(WaitsetError::ChildInvalid)` (defensive; kernel guarantees a token per F10) — else `event.tag`.
Add `use system::waitset::WaitEvent;` and extend the existing `use crate::error::{...}` list with `WaitsetError`
(match the file's single grouped `use` style at `usrlib/src/syscall.rs:2`). No usrlib runtime tests added (host
tests cannot execute `raw_syscall`; decode branches are trivially covered by the C1 totality tests).

**C3 gate:** `cargo xtask test --skip-integration` + `cargo xtask build` (the latter compiles the `cfg(not(test))`
dispatch — the only local witness for Steps 12's arms until C4's QEMU gate). Commit subject example:
`Expose waitset syscalls and usrlib wrappers`.

### Commit C4 — demo app + shell command + USER_APPS + scripted QEMU test (LD8)

**Step 14.** New workspace member `apps/waitset_demo`:
- `Cargo.toml`: add `"apps/waitset_demo",` to `members` after `"apps/test_suite",` (`Cargo.toml:22`, the last apps
  member — verified list order ends `... vfs_server, vfs_client, snake, tetris, conway, test_suite`); NOT to
  `default-members` — `ping_client` is not there either.
- `apps/waitset_demo/Cargo.toml`: mirror `apps/ping_client/Cargo.toml` verbatim-modulo-name (verified shape):
  package `name = "waitset_demo"`, `publish = false`, `edition.workspace = true`; `[[bin]] name = "waitset_demo"`,
  `bench = false`, `test = false`; `[dependencies] system = { path = "../../system" }`,
  `usrlib = { path = "../../usrlib" }`; `[target.'cfg(target_os = "none")'.dependencies]
  usrlib = { path = "../../usrlib", features = ["runtime"] }`.
- `src/main.rs` (`#![no_std]` + `extern crate alloc;`, sentinel discipline of `ping_client/src/main.rs`),
  self-contained single task, service name `"WAITSET"`:
  1. `let _ = Syscall::ipc_bind("WAITSET");` — ignore `AlreadyBound` (re-run after the first invocation keeps the
     kernel-side binding alive; connect still works).
  2. connect loop until `Ok` (precedent `connect_to_ping_server`, `ping_client/src/main.rs:34-41`).
  3. `Syscall::waitset_create()` → set.
  4. Rounds 1..=3 with tags 11/22/33, each: `let child = Syscall::ipc_accept_message(binding).expect(...)`
     — **accept on the binding, not receive on the connection (F3 binding correction)** — then
     `waitset_attach(set, child, tag)` ok, then `Syscall::ipc_send(connection, &data)` with `data[0] = round as u8`,
     rest zero (raw `ipc_send` precedent `ping_client/src/main.rs:67`), then `waitset_wait(set)` and assert
     `event_handle == child && tag_matches`, then drain the payload via the existing
     `Syscall::wait_for_message(child)` (LD4: consume + `IpcMessageFuture` downcast) and assert
     `pong.data()[0] == round as u8`; print `waitset round: {n} tag: {tag} ok` or the `fail` variant (§4).
     (Sequence note: accept→attach→send means the cascade has already recorded the event when `waitset_wait`
     runs — the round is deterministic and non-blocking; the blocking wake path is unit-proven by C2 Step 10 test 5.)
  5. Negative checks, each printing its §4 line: `waitset_attach(set, set, 0)` → expect
     `Err(WaitsetError::NestingForbidden)`; bogus well-formed-but-absent child via raw path (F4): derive the set
     word `u32::from_str_radix(&format!("{}", set), 16)` (precedent `ping_client/src/main.rs:26-28`) and call
     `usrlib::arch::raw_syscall(SyscallNum::WaitsetAttach as usize, set_word as usize, 0x7FFF_FFFF, 99)`, asserting
     the raw return equals `system::error::ErrorCode::WaitsetChildInvalid.to_reg()` (the demo may import `system` —
     ping_client does).
  6. Guard proof: `let child4 = Syscall::ipc_accept_message(binding)`; `waitset_attach(set, child4, 44)`;
     `Syscall::wait_for_message(set)` must return `Err` (guard fired, set NOT consumed); then `ipc_send`, then
     `waitset_wait(set)` still yields `child4`/tag 44 (the "subsequent waitset_wait still works" proof), drain its
     payload; print `waitset negative: waitfuture guarded ok`.
  7. `waitset_drop(set); waitset_drop(set);` → print `waitset negative: drop idempotent ok`
     (wrapper is infallible by contract; the line proves no panic/hang and shape).
  8. print `waitset complete: 3 rounds ok`; `Syscall::ipc_disconnect(connection)`; return.
Gate: `cargo xtask build` (builds the new ELF + repacks ROM per F7/F2).

**Step 15.** `apps/shell/src/shell.rs` — wiring exactly parallel to the verified `ping` precedent sites:
after `static PING_ELF` (`:30-31`) add
`#[cfg(target_arch = "x86_64")]` + `static WAITSET_ELF: &[u8] = include_bytes!("../../../target/rosx-user/release/waitset_demo");`;
after `commands.insert(String::from("ping"), ping as fn());` (`:53-54`) add
`#[cfg(target_arch = "x86_64")]` + `commands.insert(String::from("waitset"), waitset as fn());`;
after `fn ping()` (`:145-150`) add `#[cfg(target_arch = "x86_64")] fn waitset() { if let Some(fh) = Syscall::load(WAITSET_ELF) { Syscall::wait_future(fh); } }`.
Why cfg'd: mirrors `ping`/`vfs` (x86_64-only artifacts).
Risk note: `checks/compile_fail.sh` mutates this file to inject E0308 probes (map §1, `docs/todo.md:9`); adding
lines at these sites does not change the injected snippets, and the script is run by nobody in CI — if it has
drifted it is a pre-existing condition; record, do not chase, in this commit.

**Step 16.** `xtask/src/main.rs:5`: grow the quoted list per F2 to
`const USER_APPS: [&str; 12] = ["hello_elf", "random_gen_server", "snake", "tetris", "conway", "bulk_echo", "bulk_client", "ping_server", "ping_client", "vfs_server", "vfs_client", "waitset_demo"];`
(`pack_rom` picks it up automatically — ROM content grows by one ELF).

**Step 17.** New `tests-integration/tests/waitset_or.rs` (path per F5 — `tests/`, not `src/`), modeled on
`ipc_ping.rs` (import style `use tests_integration::qemu::QemuSession;`):
constants: `READY_TIMEOUT: Duration = Duration::from_secs(60);` `SWEEP_TIMEOUT: Duration = Duration::from_secs(180);`
(both values verbatim from `ipc_ping.rs:4-5`), plus the §4 sentinel constants; reuse the
`count_prefixes`/`tail_chars`/`window_diagnostics` helper shapes from `ipc_ping.rs:14-43` adapted to the two prefixes.
Single `#[test]`: `let mut session = QemuSession::spawn();` `let mark = session.mark();`
`session.expect_output("rose>", READY_TIMEOUT);` `session.send_line("waitset");`
then `session.wait_until_since(mark, |delta| delta.contains(LAST_LINE), SWEEP_TIMEOUT)` with the panic tail
diagnostic precedent (`ipc_ping.rs:76-82`); then `session.content_since(mark)` and assert: exactly 3 lines with
`ROUND_PREFIX`, each splitting into exactly 6 whitespace tokens with `tokens[0]=="waitset"`, `tokens[1]=="round:"`,
`tokens[2]` in `"1".."=3"` in order, `tokens[3]=="tag:"`, `tokens[4]` in `[11,22,33]` matching round order,
`tokens[5]=="ok"`; exactly 4 lines with `NEGATIVE_PREFIX`, each `ends_with(" ok")`, the four trailing identities in
§4 order; contains `LAST_LINE`; and for `forbidden in FORBIDDEN_STRINGS`: `!content.contains(forbidden)`
(`ipc_ping.rs:192-198` precedent). No boot-server sentinel is expected (the demo prints no boot banner —
unlike `ipc_ping`'s `"[IPC Server] Ping"`). Cross-binding statement (repeat in Step 14's comment... no comments in
code — the statement lives here and in the commit message): **the §4 strings and these test constants must agree
byte-for-byte; any divergence fails this test.**
Gate order (F5): `cargo xtask build` FIRST, then
`cargo test -p tests-integration --test waitset_or`, then regressions
`cargo test -p tests-integration --test boot_and_shell` and `cargo test -p tests-integration --test ipc_ping`.
All three QEMU gates are `unverified` scripted gates (harness exists, map §13 — never "manual"), local/verifier-tier
(CI excludes them). Commit subject example: `Prove waitset OR composition end to end in QEMU`.

### Commit C5 — documentation sweep

**Step 18.** `AGENTS.md`, bind each corrected line:
- `:42` apps list line — append `, waitset_demo` after `vfs_client` (full current text quoted in §1 F2-adjacent
  read: `[workspace members]  shell, dummy, hello_elf, random_gen_server, bulk_echo, bulk_client, ping_server, ping_client, snake, tetris, conway, test_suite, vfs_server, vfs_client`).
- `:149` subsystem invariant sentence — replace with:
  `- \`future/\` + \`ipc/\` — **notification-driven** futures (time, task-completion, keyboard, depth-one waitset aggregates) and a mailbox IPC manager (bind/connect/send/receive); a waitset completes when any attached leaf future does, woken by a child-to-waitset cascade inside \`FutureRegistry::notify\`; no polling on the preemption cycle.`
- Considered-and-unchanged (recorded, not silently dropped): `:33` already renders `future/` with a slash and stays
  accurate after the reorg; `:34` (`system/ ... syscall numbers, wire layout, ...`) is a category line that the new
  `waitset.rs` obeys without edit; `:32` handle-newtype count unchanged (LD3 adds no newtype); the header
  `> **Last Updated:** 2026-10-08` is metadata — optional bump, not bound here.

**Step 19.** `README.md`: insert one tree line after `│   ├── dummy/              # Dummy app for testing`
(`:181`, keeping `test_suite/` as the sole `└──` leaf — no glyph surgery):
`│   ├── waitset_demo/       # Waitset OR-composition demo`.
Explicit follow-up item (pre-existing gap, NOT fixed here, recorded per sweep law): the README apps tree already
omits `bulk_echo`, `bulk_client`, `ping_server`, `ping_client`, `vfs_server`, `vfs_client` as of HEAD — a separate
doc pass should reconcile the tree with the full 15-member app inventory. `docs/todo.md:9` verified unaffected.

**Step 20.** C5 gate (POSIX `grep -E` presence/absence, no `rg`):
`grep -E "waitset_demo" AGENTS.md` hits; `grep -E "waitset_demo" README.md` hits;
`grep -E "depth-one waitset aggregates" AGENTS.md` hits;
`grep -E "WaitSetKind|AND" AGENTS.md` finds nothing new beyond pre-existing text (visual diff review of the two touched files: `git diff HEAD~1 -- AGENTS.md README.md`).
Commit subject example: `Document waitset subsystem and demo app`.

---

## 6. Risks and edge cases (Coder must handle)

1. **Demo routing hang (F3):** using `ipc_receive_message` instead of `ipc_accept_message` deadlocks the demo
   (send never reaches the client mailbox from the client task). Binding correction is F3; the QEMU gate catches it.
2. **Cascade borrow discipline:** holding an arena borrow across `self.notify(parent)` or across `wake_tasks` is a
   single-threaded-reentrancy footgun (`KernelCell` has no borrow tracking — map §6). Enforce decide-inside/act-outside
   (§3.3 step 5). The restore-row-unconditionally rule must survive every early-return path.
3. **Generation recycling (map correction 6c):** the two gates in §3.7 are mandatory; without them a mailbox stale
   waiter (`mailbox_manager.rs` never prunes stale handles — map §8) could record a dead child token into a waitset
   and detonate the F6 `unwrap()` in the `WaitFuture` arm. F6 itself stays unfixed; the waitset path must not hand
   out dead tokens.
4. **Detach reverse-row leak:** forgetting the `parents_of` row removal in `detach` (or in `drop_waitset`'s child
   pass) records events for edges the user removed — live parents make the cascade unable to self-heal. Tests 11 and 12 pin this.
5. **Spurious wakes:** broadcast wake + steal-by-loop means a woken waiter can find the queue already drained or
   stale-filtered; the `wait_waitset` loop re-blocks. Never treat a wake as an event.
6. **Blocked-on-removed-mailbox hole (map correction 6b):** a child whose mailbox disconnects is never notified; the
   waitset simply never records for it — pre-existing lifecycle hole inherited untouched; the demo's connection stays
   alive until after the last drain; do not "fix" it here.
7. **Re-running the shell command:** `"WAITSET"` stays bound after the first run; the bind result is ignored per
   Step 14 (AlreadyBound is benign). The test runs it exactly once.
8. **Sentinel drift:** §4 strings are a cross-file byte-binding; the forbidden substrings `fail`/`Panic` must not
   appear in any ok-line or shell echo (`waitset` is clean).
9. **LD1/LD2 vocabulary:** the final grep `grep -E "WaitSetKind|MAX_RANK|rank" kernel/src/future/waitset.rs` must find
   nothing — no identifier, comment (there are none anyway), or test name may contain `rank` in that file.
10. **ROM/image growth:** `USER_APPS` 11→12 grows the ROM image; `pack_rom` and `main.rs:70` include_bytes handle it;
    if disk-image sizing were an issue, `cargo xtask build` fails loudly — it is a CI-proven form but not proven for
    +1 app until C4 runs it.
11. **`cargo xtask test` ordering dependency:** the new shell `include_bytes!` needs `target/rosx-user/release/waitset_demo`,
    which `xtask test`'s `build_apps()` guarantees only after Step 16 landed — do not split Steps 15/16 across commits.
12. **x86_32:** transitive compile only (F8); no gate bound; if it were ever broken, the fix is the same arms — they are arch-neutral.

## 7. Blast radius (test processes proven, not assumed)

C2 rewrites shared state reached through the global service registry (`services().future_registry`) behind
constructor-wired plumbing (`KernelServices::init`, `kernel_services.rs:81-125`, incl. the `wake_controller` dyn
wiring at `:89-94` and `wire_scheduler_for_tests`, `scheduler/mod.rs:49-64`). By map §10 the `#[cfg(test)]` test
processes that transitively reach `notify` / `register_waiter` / `consume` / `get` — enumerated there (line-cited
there, test totals deliberately not asserted here; the gate is zero failing tests):
`kernel/src/future.rs` (moved to `future/mod.rs`; `notify_moves_waiters_to_ready`, `notify_does_not_double_wake`,
`future_registry_can_complete_time_future_via_trait`, `complete_default_noop_does_not_affect_other_futures`, TimeFuture tests);
`kernel/src/kernel_services.rs` (`init_and_access_services`);
`kernel/src/ipc/mailbox_manager.rs` (`test_pop_front_async`, `test_push_back_notifies_waiters` — the wake-through-real-scheduler witness);
`kernel/src/ipc/ipc_manager.rs` (the 10 `manager_with_notifier()` tests + `delivered_message` reach the registry;
the 4 plain `IpcManager::new()` data-family tests do not);
`kernel/src/scheduler/scheduler.rs` (wake-side contract incl. `wake_tasks_should_not_resurrect_terminated_task`);
`kernel/src/scheduler/mod.rs` (real factories + services); `kernel/src/task.rs` / `task_manager.rs` touch only
completion-future storage. All of these execute under the single gate `cargo xtask test --skip-integration`
(host process, `--test-threads=1` per map §12). The **production process kind** (cross-target kernel in QEMU) is
executed by `cargo xtask build` at C2/C3 and by the three C4 QEMU gates: `boot_and_shell` exercises the keyboard-future
notify path and `ipc_ping` the mailbox notify path — both are the untouched-cascade-continuation regressions, and
`waitset_or` exercises the cascade itself. No other process kind executes these paths (x86_32 is compile-only, F8).

## 8. Acceptance criteria (exact; run in this order at branch end)

1. `cargo xtask test --skip-integration` — zero failing tests (CI-proven form; re-run, all C2/C3 tests included).
2. `cargo xtask build` — succeeds (CI-proven form; includes new app, ROM, kernel, disk image).
3. `cargo test -p tests-integration --test waitset_or` — scripted QEMU gate (`unverified` until first green; local/verifier-tier, CI-excluded).
4. `cargo test -p tests-integration --test boot_and_shell` — regression (`unverified`, same tier).
5. `cargo test -p tests-integration --test ipc_ping` — regression (`unverified`, same tier).
6. Wire-freeze greps (POSIX `grep -E`, all must behave as stated):
   - `grep -E "^    RomRead = 27,$" system/src/syscall_numbers.rs` — hits.
   - `grep -E "^            27 => Ok\(Self::RomRead\),$" system/src/syscall_numbers.rs` — hits.
   - `grep -E "IpcServerNotFound = 0x0101|IpcDataNotReadable = 0x010C|Unknown = 0x7FFF_FFFF" system/src/error.rs` — hits, values verbatim per §1 F10; and
     `git diff da8d54b..HEAD -- system/src/syscall_numbers.rs | grep -E "^-[^-]"` produces **no output** (pure append;
     the `[^-]` guard is load-bearing: a bare `^-` always matches the diff's own `--- a/...` header line, which would
     make the gate unsatisfiable by construction),
     while for `system/src/error.rs` the only permitted removal-line in the diff is the single `; 13]` array-length
     line replacement in the test mod (`    const ALL_ERROR_CODES: [ErrorCode; 13] = [` -> `; 17]`; verified: exactly
     one `; 13]` occurrence in that file) — no existing enum value or `from_code` arm modified or deleted.
7. Regression guards: `grep -E "WaitSetKind|MAX_RANK|rank" kernel/src/future/waitset.rs` finds nothing (exit 1 — LD1/LD2);
   `grep -E "waitset_demo" xtask/src/main.rs` hits.
8. Warning gate: **Δ-new = 0** against the verifier-reestablished warning baseline fingerprint captured at the
   branch-point commit `da8d54b` (the plan's self-attestation and any prior capture are not evidence; the
   fingerprint must be taken on that commit before C1). Capture per commit with a filter matching BOTH warning
   forms — bare and code-bearing: `cargo xtask build 2>&1 | grep -E "^warning(:|\[)"` — and arbitrate Δ-new by
   location first (a diagnostic located only in unchanged files is baseline, not new); totals corroborate but never
   convict. Decreases at legitimately deleted sites are expected and attributed by baseline multiset diff, never
   treated as violations (this change deletes no warning sites, so expect Δ-new = 0 and Δ-total = 0 with locations identical).
9. Commit discipline: five commits C1–C5, each green at its own boundary; subjects bare imperative, no
   conventional-commit prefix (repo style, map §14).
10. Never invoked: `cargo build --workspace` (unsupported by design — AGENTS.md).
