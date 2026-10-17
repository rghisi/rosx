# IPC Bulk Data Transfers — Implementation Plan

> **Date:** 2026-10-15 · **Branch:** `ipc-bulk-data-transfers` @ `28288f2` · **Spec:** `plans/ipc-bulk-data-transfers.md` (LOCKED — no decisions re-opened)
> **Inputs:** Context Map `.opencode/artifacts/ipc-bulk-context-map.md` (all claims relied on below re-verified against source on 2026-10-15), baseline `.opencode/artifacts/ipc-bulk-baseline.md` (four gates GREEN verbatim on `28288f2`).
> **Toolchain facts (baseline-verified):** cargo 1.101.0-nightly, QEMU at `/usr/bin/qemu-system-x86_64`.

---

## 0. Validated baseline (what was proven, not assumed)

| Gate (verbatim, cwd) | Verdict on `28288f2` | Recorded evidence |
|---|---|---|
| `cargo test -p kernel -- --test-threads=1` (repo root) | PASS | 224 passed, 0 failed |
| `cargo xtask test` (repo root) | PASS | 260 host + 7 QEMU tests, 0 failed |
| `cargo xtask build` (repo root) | PASS | disk image created |
| `cargo build -p rosx-x86` (cwd `arch/x86_32/`) | PASS | real compile; `rosx-x86` 1 warning (`unused_unsafe`) |

Baseline warning fingerprint (Δ-new reference, per-target): kernel lib **60**, kernel lib-test **86**, shell lib **3**, tests-integration lib **1**, `rosx` bin **26**, `rosx` manifest **1**, `rosx-x86` bin **1**. These are baseline claims sourced from the delegation's verifier transcript, not from any prior plan.

Acceptance wording for warnings throughout this plan: **zero NEW warnings vs this fingerprint (Δ-new = 0)**, counting both bare `warning:` and code-bearing `warning[EXXXX]:` lines; decreases at legitimately deleted sites are expected, never violations. Per-step gates below are exit-code gates; the Δ-new arbitration lives in Step 7.

---

## 1. Binding Resolutions (BR) — the coder follows these verbatim

- **BR-1 (Treaty form).** `SyscallNum` conversion is `TryFrom<usize>` (`system/src/syscall_numbers.rs:23-48`, hand-written match ending `_ => Err(())`), **not** `TryFrom<u8>`. New variants `IpcDataWrite = 24`, `IpcDataRead = 25`, `IpcDataRelease = 26` go into **both** the enum and the usize match arms. The `_ => 0` fallthrough in `handle_syscall` (`kernel/src/syscall.rs:122`) hides a missing TryFrom arm silently — never omit either.
- **BR-2 (Four mirrored lists).** The four new codes `IpcDataTooLarge = 0x0109`, `IpcDataBudgetExceeded = 0x010A`, `IpcDataNotFound = 0x010B`, `IpcDataNotReadable = 0x010C` grow **all four** test lists in the same commit that adds them: `system/src/error.rs:45` `ALL_ERROR_CODES: [ErrorCode; 9]` → `[ErrorCode; 13]`; `kernel/src/handle.rs:217` same; `usrlib/src/error.rs:104` same; and the inline array in the test `packed_handle_space_is_disjoint_from_error_space` (`system/src/ipc.rs:56-66`).
- **BR-3 (Exhaustive-match ordering — plan finding).** The `match SyscallNum::try_from(num)` in `kernel/src/syscall.rs:15-123` enumerates **every** variant with no wildcard (last arm `Err(_) => 0`). Adding enum variants without dispatch arms is compile error E0004 in every non-test kernel build (gates 3 and 4; gate 1 alone stays green because `handle_syscall` is `#[cfg(not(test))]`). Resolution: **Step 1 lands three stub arms** `Ok(SyscallNum::IpcDataWrite | SyscallNum::IpcDataRead | SyscallNum::IpcDataRelease) => ErrorCode::Unknown.to_reg(),` (fail-closed, matching the `IpcReceiveMessage`/`IpcAcceptMessage` malformed-token convention at `syscall.rs:116,120`); **Step 4 replaces the stubs** with real dispatch. Do **not** "fix" this with a catch-all `_ =>` arm — that recreates the silent-fallthrough hazard BR-1 warns about.
- **BR-4 (Disconnect-GC scan — Context Map C2/C6, delegated gap 1).** `GenerationalArena` (`kernel/src/collections/arena.rs:11-78`) exposes only handle-addressed ops (`new/add/borrow/borrow_mut/remove/replace`) and **stays untouched** (shared infrastructure; smaller blast radius). Binding choice: a **secondary index inside `IpcDataManager`**: `by_connection: BTreeMap<IpcConnectionHandle, alloc::vec::Vec<Handle>>`, maintained exclusively by `write` (insert), `evict` (remove), and `gc_connection` (drain). `DataObject.connection` is the **ground truth**: `gc_connection` validates every indexed handle by borrowing the object and confirming `object.connection == key` before removal; a mismatched or stale index entry is dropped without freeing. All arena removals for data objects route through **one private method** `fn evict(&mut self, handle: Handle) -> Option<DataObject>` — `release_data` and `gc_connection` are its only callers; this single-path invariant is what the drift tests (Step 3) prove.
- **BR-5 (R4 authorization operationalization — Context Map C4).** There is no stored server TaskHandle (`IpcConnection` stores only `client_task`, `ipc_manager.rs:40-44`). Membership is the `send_message` rule verbatim from `ipc_manager.rs:126-135` (`caller == conn.client_task` distinguishes the client endpoint; "not the client" is the server seat). Binding predicates, exactly:
  - **write** (caller→seat): `self.connections.borrow(connection.into())` must succeed; caller is client iff `caller == conn.client_task`, else the server seat (identical to `send_message`; the trust model of R4 is unchanged).
  - **read**: `IpcDataRead` requires `caller != object.writer` **and** caller on that connection, else `IpcDataNotReadable`. "On that connection" under the stored-fields constraint binds to: connection borrow succeeds **AND** (`caller == conn.client_task` **OR** `object.writer == conn.client_task`) — i.e., exactly one of (caller, writer) holds the client seat; the other is the peer. This rejects a non-client intruder on a server-written object while admitting the legitimate client/server pair both directions.
  - **release**: writer only. Unknown/stale token → `IpcDataNotFound`. Release by a non-writer caller → **`IpcDataNotFound`** (opaque; `IpcDataNotReadable` is reserved for the read path per spec R4 wording).
- **BR-6 (Unit-test target — Context Map C5).** `handle_syscall` has no `#[cfg(test)]` twin, so all R11 tests target `IpcManager` / `IpcDataManager` APIs directly. Fixtures: the `CLIENT_TASK/SERVER_TASK = TaskHandle::new(n, 1)` constants (`ipc_manager.rs:170-171`) and `bind_and_connect()` (`ipc_manager.rs:182-186`). Data tests need no futures → construct the manager with standalone `IpcManager::new()` (noop notifier, `ipc_manager.rs:56-64`); do not use `manager_with_notifier()` unless a test asserts future behavior. The stale-generation test models `send_on_recycled_connection_slot_rejected` (`ipc_manager.rs:242-264`). Compile-verification of the syscall **arms** (cfg(not(test)) code) is carried by Gate 4 (`cargo build -p rosx-x86` compiles `kernel` non-test and thus `handle_syscall`) and Gate 3.
- **BR-7 (UserDataHandle Drop — delegated gap 2).** `UserDataHandle` is non-`Copy`, non-`Clone` (Drop forbids Copy — Context Map C8, a deliberate break from the three `Copy` usrlib siblings). Silent-failure site: a writer whose peer already disconnected gets an error (or NotFound) from the release syscall issued inside `Drop`; the drop must not panic and must not leak (R6 disconnect-GC is the backstop). Resolution without comments: the token is `Option<u32>`; explicit `release(&mut self)` does `Option::take` first, and `Drop::drop` is exactly `let self::release(&mut self);` → `let _ = self.release();`. The `_` binding is the self-evident ignore; `Option::take` makes explicit-then-drop a no-op (no double release, no double refund). **Host-test hazard (verified: `usrlib/src/arch/mod.rs` cfg-selects the real `syscall`/`int 0x80` by `target_arch`, so a host `raw_syscall` executes a real Linux syscall):** usrlib unit tests must **never construct or drop a live (`Some`) `UserDataHandle`**; handle-side host tests construct only `UserDataHandle { packed: None }` (child test module can build the private-field literal) and exercise the take-guard; the live path is proven by the Step-6 QEMU run, not host tests.
- **BR-8 (R12 topology).** `bulk_echo` (server) **and** `bulk_client` are separate ELF apps scheduled by the kernel from `arch/x86_64/src/main.rs` (the `random_gen_server` precedent at `main.rs:74-75`). The client is **not** a shell command: `apps/shell/src/shell.rs:10-28` `include_bytes!`s are `#[cfg(target_arch = "x86_64")]` / `#[cfg(target_arch = "x86")]` pairs, so a shell-routed client would make a `target/rosx-i686-user/release/bulk_client` ELF a Gate-4 precondition (Context Map §11 trap) — avoided entirely. `arch/x86_64/build.rs` panics on missing embedded ELFs (`build.rs:9-11`) and must be extended in the same commit as the embedding. The client retries `ipc_connect("BULKECHO")` until `Ok` (boot-order independence). Markers are defined in Step 6 and asserted by `tests-integration/tests/ipc_bulk.rs` modeled on `ipc_random.rs`.
- **BR-9 (Depot rules).** Kernel deps stay `system` + `lazy_static` (`kernel/Cargo.toml:8-10`); `Vec`/`BTreeMap` come from `alloc` (already used in `ipc_manager.rs:1-3`). No new deps anywhere. `system` gains numbers/codes only — no Data types (R10).
- **BR-10 (Repo law).** No comments anywhere (AGENTS.md CRITICAL; asm exempt). Every kernel module touched ships `#[cfg(test)]` coverage. Kernel unit tests run with `-- --test-threads=1` only (process-global services). Commit messages carry **no prefix**.
- **BR-11 (Error projections beyond the locked four codes).** `DataError` lives in `kernel/src/ipc/errors.rs` (family convention): variants `ConnectionNotFound, TooLarge, BudgetExceeded, NotFound, NotReadable`. `From<DataError> for ErrorCode`: `ConnectionNotFound → ErrorCode::IpcSendConnectionNotFound` (reuse of the existing 0x0104 — exactly what `send_message` returns for that condition; no new wire number), and the other four → the four locked codes. A syscall edge receiving a non-token argument word (`from_packed → None`) returns `ErrorCode::IpcDataNotFound.to_reg()` (write path with a bad conn word returns `ErrorCode::IpcSendConnectionNotFound.to_reg()`). Arena capacity exhaustion (`arena.add` → `Error::OutOfMemory`) maps to `DataError::BudgetExceeded` (resource-limit-exhausted semantics; reachable when payloads are tiny). `IpcDataRead` success is a plain byte count `< ERROR_BIT` (≤ 65536 « 2³¹ — codec-safe).

---

## 2. Implementation steps

Per-step gates use only baseline-proven command forms (the four gates above) plus `cargo xtask test --skip-integration` (flag exists at `xtask/src/main.rs:79-87,146`, is what CI runs per Context Map §8, and is Gate 2 minus the QEMU binaries) and `cargo xtask apps` (`xtask/src/main.rs:116-121`, byte-identical to the `build_apps()` phase proven inside Gates 2/3). All gates run from the stated cwd. Each step ends with an individual commit (no prefix).

### Step 1 — `system` treaty numbers + kernel compile-order stubs (spec increment 1)

**Files:** `system/src/syscall_numbers.rs`, `system/src/error.rs`, `system/src/ipc.rs`, `kernel/src/handle.rs` (test mod only), `usrlib/src/error.rs` (test mod only), `kernel/src/syscall.rs` (stub arms).

- **Modify** `system/src/syscall_numbers.rs`: append to the enum (after `IpcAcceptMessage = 23`, line 20): `IpcDataWrite = 24`, `IpcDataRead = 25`, `IpcDataRelease = 26`. Append to the `TryFrom<usize>` match (before `_ => Err(())`, line 45): `24 => Ok(Self::IpcDataWrite),` `25 => Ok(Self::IpcDataRead),` `26 => Ok(Self::IpcDataRelease),` (BR-1; numbers verified unused).
- **Modify** `system/src/error.rs`: insert the four variants before `Unknown = 0x7FFF_FFFF` (`:18`) with values `0x0109..=0x010C`; add the four `from_code` arms (`:26-38`) before `_ => ErrorCode::Unknown`; grow `ALL_ERROR_CODES` (`:45`) to `[ErrorCode; 13]` adding the four entries in code order before `Unknown`. Existing tests (`every_error_code_variant_roundtrips_through_the_register_codec`, totality test) then prove the codec for the new codes with no new test fns required.
- **Modify** `system/src/ipc.rs`: add the four codes to the inline disjointness array in `packed_handle_space_is_disjoint_from_error_space` (`:56-66`).
- **Modify** `kernel/src/handle.rs` test mod: grow `ALL_ERROR_CODES` (`:217`) to `[ErrorCode; 13]` (BR-2).
- **Modify** `usrlib/src/error.rs` test mod: grow `ALL_ERROR_CODES` (`:104`) to `[ErrorCode; 13]` (BR-2). The existing mirror-mapping tests use `_ =>` catch-alls, so the four new codes flow through without behavior change and the grown array proves totality.
- **Modify** `kernel/src/syscall.rs`: add the three-arm stub of BR-3 immediately after the `Ok(SyscallNum::IpcAcceptMessage)` arm (ends `:121`), before `Err(_) => 0`. Body: `ErrorCode::Unknown.to_reg()`. This is required for compilability — without it Gates 3/4 die with E0004.
- **Why:** the whole feature hangs off frozen numbers; landing them atomically with all mirrored lists prevents silent coverage rot (C3) and stubs keep every intermediate commit building (BR-3).
- **Verify:** `cargo xtask test --skip-integration` (repo root) → exit 0 (system 7 + grown-array sweeps, kernel 224, usrlib 6, all ok). `cargo build -p rosx-x86` (cwd `arch/x86_32/`) → exit 0 (shell→usrlib→system chain, §11; stub arms compile).
- **Commit:** `Add IPC data transfer syscall numbers and error codes to treaty`

### Step 2 — `IpcDataHandle` newtype (spec increment 2)

**Files:** `kernel/src/handle.rs`.

- **Add** the fifth newtype `IpcDataHandle(Handle)` after `IpcBindingHandle` (ends `:209`), copying that block verbatim: same derives (`Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Debug`), `#[allow(dead_code)]` impl with `new/pack/from_packed/index/generation`, and both `From<Handle>`/`Into<Handle>` directions (required by the arena — `arena.rs:32-77` speaks bare `Handle`).
- **Add** roundtrip/domain tests mirroring the existing per-type coverage: pack bit-layout, roundtrip + edge values (`(0,0)…(65535,32767)`), the exhaustive "pack never sets ERROR_BIT" sweep style (`:256-273`), `from_packed` garbage rejection (`:275-283`), and one `IpcDataHandle`-typed domain-panic test under `#[cfg(debug_assertions)]` like `:306-311`. Reuse `ALL_ERROR_CODES` disjointness as-is (already 13-wide from Step 1).
- **Why:** R8 — the kernel-side token is the shared handle family's member; the private-field shape is what keeps bit 31 out of user-visible words.
- **Verify:** `cargo test -p kernel -- --test-threads=1` (repo root) → exit 0, total > 224 (new tests present and running).
- **Commit:** `Add IpcDataHandle newtype to kernel handles`

### Step 3 — `IpcDataManager`: storage, budget, index, GC, unit suite (spec increment 3)

**Files:** `kernel/src/ipc/mod.rs`, `kernel/src/ipc/data_manager.rs` (new), `kernel/src/ipc/errors.rs`, `kernel/src/ipc/ipc_manager.rs`.

- **Modify** `kernel/src/ipc/mod.rs`: add `pub(crate) mod data_manager;` (one line; file is 5 lines).
- **Modify** `kernel/src/ipc/errors.rs`: add `DataError` (BR-11) + `impl From<DataError> for ErrorCode` + a mapping test following the file's per-family convention (`:65-141` patterns — iterate variants, assert `to_reg` bit and `from_code` decode).
- **Add** `kernel/src/ipc/data_manager.rs`:
  - `struct DataObject { payload: alloc::vec::Vec<u8>, connection: IpcConnectionHandle, writer: crate::task::TaskHandle, read_cursor: usize }` (`TaskHandle` imported from `crate::task` per `task.rs:10`).
  - `pub(crate) struct IpcDataManager { objects: GenerationalArena<DataObject, 256>, outstanding: BTreeMap<TaskHandle, usize>, by_connection: BTreeMap<IpcConnectionHandle, Vec<Handle>> }`.
  - Constants `const MAX_DATA_BYTES: usize = 64 * 1024;` and `const DATA_BUDGET_BYTES: usize = 256 * 1024;`.
  - `pub(crate) fn write(&mut self, caller: TaskHandle, connection: IpcConnectionHandle, payload: &[u8]) -> Result<IpcDataHandle, DataError>` — order of checks: `payload.len() > MAX_DATA_BYTES` → `TooLarge`; `outstanding[caller] + len > DATA_BUDGET_BYTES` → `BudgetExceeded` (checked **before** allocating, R5 "cannot exhaust kernel heap"); `arena.add` error → `BudgetExceeded` (BR-11). On success: snapshot copy (`Vec::from(payload)`), cursor 0, budget add, index insert, return the packed-able handle.
  - `pub(crate) fn read(&mut self, caller: TaskHandle, token: IpcDataHandle, dst: &mut [u8]) -> Result<(usize, IpcConnectionHandle, TaskHandle), DataError>` — storage-side part of read: borrow object (`Error::NotFound` → `DataError::NotFound`, covers stale generation); `caller == object.writer` → `NotReadable`; copy `min(dst.len(), remaining)` at `read_cursor`, advance cursor, return `(bytes_copied, object.connection, object.writer)` so `IpcManager` can run the BR-5 connection-membership gate **after** the copy-side facts are known — **correction for compilability the coder must follow instead**: do **not** copy before membership is verified; split into `fn locate(&self, caller, token) -> Result<(IpcConnectionHandle, TaskHandle), DataError>` (borrow + writer check) and `fn copy_out(&mut self, token, dst) -> usize` (pure cursor copy). `IpcManager::read_data` (below) orders: `locate` → connections borrow + BR-5 membership (`NotReadable` on failure) → `copy_out`. End-of-data read returns `Ok(0)`; zero-length `dst` returns `0`.
  - `pub(crate) fn release(&mut self, caller: TaskHandle, token: IpcDataHandle) -> Result<(), DataError>` — borrow object; missing/stale → `NotFound`; `caller != object.writer` → `NotFound` (BR-5); then `self.evict(...)`.
  - `pub(crate) fn gc_connection(&mut self, connection: IpcConnectionHandle)` — take `by_connection[connection]`; per indexed handle: arena borrow (stale → drop entry silently), assert `object.connection == connection` (ground truth, BR-4; mismatch → drop entry without freeing), refund `outstanding[object.writer]` by `object.payload.len()` **read before removal**, `evict`. Remove emptied index keys.
  - `fn evict(&mut self, handle: Handle) -> Option<DataObject>` — the single removal path (BR-4): arena `remove`, scrub the handle from every `by_connection` value (`Vec::retain`-style by value; capacity is 256 — cost irrelevant), return the object.
- **Modify** `kernel/src/ipc/ipc_manager.rs`:
  - `IpcManager` gains field `data_manager: IpcDataManager` (`new()` and `new_with_notifier()` both initialize it — `:56-74`).
  - `disconnect` (`:112-119`): after `let _ = self.connections.remove(connection_handle.into());` add `self.data_manager.gc_connection(connection_handle);` — same success path; the stale-handle early return at `:115` correctly skips GC (that slot's objects were GC'd by the real disconnect) (C6).
  - New `#[allow(dead_code)]` impl block (the repo's precedent for not-yet-dispatched kernel API — `handle.rs:46,88,130,170`) with the three public operations; keep the attribute through Step 4 to hold the warning fingerprint flat:
    - `pub(crate) fn write_data(&mut self, caller: TaskHandle, connection: IpcConnectionHandle, payload: &[u8]) -> Result<IpcDataHandle, DataError>` — BR-5 write seat rule: `self.connections.borrow(connection.into()).map_err(|_| DataError::ConnectionNotFound)?;` then delegate to `data_manager.write`.
    - `pub(crate) fn read_data(&mut self, caller: TaskHandle, token: IpcDataHandle, dst: &mut [u8]) -> Result<usize, DataError>` — `locate` → `connections.borrow(connection.into()).map_err(|_| DataError::NotReadable)?` → membership `caller == conn.client_task || writer == conn.client_task` else `NotReadable` (the `caller == conn.client_task` test is the verbatim pattern from `:126-135`) → `copy_out`.
    - `pub(crate) fn release_data(&mut self, caller: TaskHandle, token: IpcDataHandle) -> Result<(), DataError>` — delegate to `data_manager.release`.
  - **Add** `#[cfg(test)]` coverage — in `data_manager.rs` tests (storage/budget/index; construct `IpcDataManager` directly, fake `IpcConnectionHandle::new(_, 1)` / `TaskHandle::new(_, 1)` constants, private-field assertions per the `arena.rs:80+` precedent):
    1. `write_read_roundtrip_returns_full_payload`
    2. `short_read_then_continuation_advances_cursor` (two reads split a payload; byte-exact)
    3. `read_past_end_returns_zero`
    4. `read_with_empty_dst_returns_zero`
    5. `write_over_max_data_bytes_is_too_large` (payload `MAX_DATA_BYTES + 1`)
    6. `budget_exhaustion_then_refund_after_release` (fill `DATA_BUDGET_BYTES` in 64 KiB slices, next write `BudgetExceeded`, after a release a same-size write succeeds)
    7. `budget_is_per_task` (writer B's writes never consume writer A's budget)
    8. `arena_capacity_exhaustion_maps_to_budget_exceeded` (256 × 1-byte writes fill the arena — budget math: 256 B ≪ 256 KiB; the 257th is `BudgetExceeded` via capacity, proving BR-11)
    9. `double_release_is_not_found`
    10. `release_by_non_writer_is_not_found`
    11. `writer_cannot_read_own_object_is_not_readable`
    12. `stale_generation_token_rejected_after_slot_reuse` — models `send_on_recycled_connection_slot_rejected` (`ipc_manager.rs:242-264`): write h1 (1 byte), release h1, 255 × (write + release), write h2; primary assertion `read(INTRUDER, h1_token)` → `NotFound` after the slot recycled; parity assertions `h2.index() == h1.index()` and `h2.generation() == h1.generation() + 1`.
    13. `index_entries_always_borrow_live_objects` — after a fixed deterministic script (writes on 2 connections, releases, one `gc_connection`): every handle in every `by_connection` value borrows in the arena **and** `object.connection == map key` (the BR-4 no-drift proof; relies on release flowing through one path only).
    14. `release_scrubs_index_and_refunds` (index Vec shrinks; `outstanding` decrements by payload length).
    15. `many_concurrent_objects_per_connection` (8 live objects, one connection, interleaved cursors independent).
  - In `ipc_manager.rs` `mod tests` (use `IpcManager::new()` + `bind_and_connect`, BR-6):
    16. `disconnect_gc_frees_objects_and_refunds_budget` — client writes 2 objects on the connection, disconnects; both tokens `read` → `NotFound`; a fresh 64 KiB write from the freed writer budget succeeds on a reconnected connection (refund proof).
    17. `write_on_dead_connection_is_connection_not_found`.
    18. `read_membership_rejects_non_client_intruder` — writer = `SERVER_TASK` (server seat, write permitted per `send_message` parity), reader = `INTRUDER = TaskHandle::new(3, 1)` (non-client, not writer) → `NotReadable`; client reader on the same object → `Ok`.
    19. `client_written_object_readable_by_server_task` (the `caller != writer` + client-seat path).
- **Why:** all R3–R6 semantics live here; the index is the only new mutable state and its correctness story is "ground truth in the object, index only locates, one eviction path".
- **Verify:** `cargo test -p kernel -- --test-threads=1` → exit 0, total ≥ 224 + 19 new, 0 failed. `cargo build -p rosx-x86` (cwd `arch/x86_32/`) → exit 0 (kernel is §11-transitive; the `#[allow(dead_code)]` keeps Δ-new = 0 for the undispatched methods).
- **Commit:** `Add IpcDataManager with budget accounting and disconnect cleanup`

### Step 4 — real syscall dispatch (spec increment 4)

**Files:** `kernel/src/syscall.rs` only.

- **Replace** the Step-1 stub arms (BR-3) with three arms:
  - `Ok(SyscallNum::IpcDataWrite)`: decode `IpcConnectionHandle::from_packed(arg1)` — `None` → `ErrorCode::IpcSendConnectionNotFound.to_reg()` (BR-11); build the user slice with the **Print-arm copy-in pattern verbatim from `:16-20`** — `unsafe { core::slice::from_raw_parts(arg2 as *const u8, arg3) }` (flat identity-mapped space, no pointer validation, same posture as `Print`); caller identity via `kernel().execution_state.current_task()` (the `:89/:109` precedent); `services().ipc_manager.borrow_mut().write_data(...)`; `Ok(handle) => handle.pack()`, `Err(e) => ErrorCode::from(e).to_reg()`.
  - `Ok(SyscallNum::IpcDataRead)`: `IpcDataHandle::from_packed(arg1)` — `None` → `ErrorCode::IpcDataNotFound.to_reg()`; dst slice `unsafe { core::slice::from_raw_parts_mut(arg2 as *mut u8, arg3) }` (copy-**out** posture per the `WaitFuture` `core::ptr::write` precedent at `:56`); `read_data` → `Ok(n) => n` (plain count, never ERROR_BIT-set per BR-11), `Err(e) => ErrorCode::from(e).to_reg()`.
  - `Ok(SyscallNum::IpcDataRelease)`: `from_packed` `None` → `ErrorCode::IpcDataNotFound.to_reg()`; `release_data` → `Ok(()) => 0`, `Err(e) => ErrorCode::from(e).to_reg()`.
- **No arch changes** (Context Map §10 verified: `syscall.S` / `int80_handler` pass numbers/args through; three args fit both ABIs).
- **Why:** the only registration points for a syscall are the TryFrom match and this dispatch (Context Map item 9/10); the stub→real swap completes the kernel surface with zero new registration points.
- **Verify:** `cargo test -p kernel -- --test-threads=1` → exit 0 (regression; note the arms are `cfg(not(test))` — this gate alone does **not** compile them). `cargo build -p rosx-x86` (cwd `arch/x86_32/`) → exit 0 — **this is the compile gate for the arms** (non-test kernel build, BR-6). `cargo xtask build` (repo root) → exit 0 (`rosx` bin carries the same code).
- **Commit:** `Dispatch IPC data syscalls to IpcDataManager`

### Step 5 — `usrlib`: `UserDataHandle`, error mirror, wrappers, conveniences (spec increment 5)

**Files:** `usrlib/src/handle.rs`, `usrlib/src/error.rs`, `usrlib/src/syscall.rs`, `usrlib/src/ipc.rs`.

- **Add** `UserDataHandle` to `usrlib/src/handle.rs` per BR-7: `pub struct UserDataHandle { packed: Option<u32> }` — non-`Copy`/non-`Clone`; `pub(crate) fn packed(self) -> u32` (returns `self.packed.unwrap_or(0)`); `pub(crate) fn from_token(raw: usize) -> Option<Self>` gated by `is_token` (sibling shape `:40-52`); hand-written `Debug`/`Display` printing `{:08x}` of the token or a fixed `"released"`-style word for `None` (hex-only, no decomposition, R8/R13); `pub fn release(&mut self) -> Result<(), DataError>` — `take()` first; `None` → `Ok(())`; else `raw_syscall(IpcDataRelease, raw, 0, 0)` and translate via the mirror; `impl Drop` — body exactly `let _ = self.release();` (BR-7: the ignore is self-evident, no comment).
- **Add** to `usrlib/src/error.rs`: `pub enum DataError { TooLarge, BudgetExceeded, NotFound, NotReadable }` + `impl From<ErrorCode>` (explicit arms for the three non-terminal codes, catch-all `_ => Self::NotFound`) + `Display` + mapping arms added to the existing mirror-totality test `every_error_code_maps_through_every_mirror_totally` (iterates the 13-wide `ALL_ERROR_CODES` from Step 1) and the fallback test.
- **Add** to `usrlib/src/syscall.rs` (wrapper style = `ipc_send` at `:121-137`: raw call → `is_token` split → mirror error):
  - `pub fn ipc_data_write(connection: UserConnectionHandle, payload: &[u8]) -> Result<UserDataHandle, DataError>` — `raw_syscall(IpcDataWrite, connection.packed() as usize, payload.as_ptr() as usize, payload.len())`; `is_token(raw)` → `UserDataHandle::from_token(raw)` `Ok`/`Err(DataError::NotFound)`-impossible split (follow the `ipc_connect` `:94-101` pattern); else `Err(DataError::from(ErrorCode::from_code(raw)))`.
  - `pub fn ipc_data_read(token: usize, dst: &mut [u8]) -> Result<usize, DataError>` — **raw usize token in** (the reader side never constructs a `UserDataHandle` — constructing one would `Drop`-release a token the reader cannot release); **no input pre-validation** (the kernel is the authority — R13); `is_token(raw)` → `Ok(raw)` else `Err(DataError::from(ErrorCode::from_code(raw)))`.
  - `ipc_data_release` is not public — `UserDataHandle::release()` is the sole release surface.
- **Add** to `usrlib/src/ipc.rs` (canonical convenience only — payload framing remains application protocol, R10):
  - Encoding contract (bound here, testable as pure fns): token is `u32` little-endian-equivalent via `to_ne_bytes` at payload **offset 0, 4 bytes** (mirrors `ipc_send_value`'s `:26-30` idiom); expose crate-private pure helpers `encode_data_token` / `decode_data_token` over `&[u8; MESSAGE_PAYLOAD_BYTES]` so the codec is unit-testable without any syscall (BR-7 host-test rule).
  - `pub fn send_data(connection: UserConnectionHandle, payload: &[u8]) -> Result<UserDataHandle, SendDataError>` with `pub enum SendDataError { Data(DataError), Send(IpcSendError) }` (+ `Display`, `From`s): `ipc_data_write` → build `[u8; 60]` with token at offset 0 → `Syscall::ipc_send`; a failed send lets the just-obtained handle fall (Drop auto-releases — no leak, no clone ever required: ownership flows in, handle ownership flows out).
  - `pub fn receive_data(message: &ReceivedMessage) -> Result<alloc::vec::Vec<u8>, DataError>`: `decode_data_token(message.data())` → loop `ipc_data_read(token, &mut [0u8; 4096])` extending the Vec until a read returns `0` (EOF by spec; bounded by `MAX_DATA_BYTES`).
- **Tests (host-safe):** handle module — `packed: None` construction, `release()` on it `Ok(())` with no syscall fired, double-release `Ok(())`; codec helpers (encode→decode identity, offset pinned at 0); error-mirror arms. **Never** a live `UserDataHandle` in host tests (BR-7).
- **Why:** R9 surface; the Drop design closes the double-release/refund hole and the no-handle-for-readers rule keeps reader ergonomics free of a release they cannot perform.
- **Verify:** `cargo xtask test --skip-integration` → exit 0 (usrlib lib-test count grows past 6). `cargo build -p rosx-x86` (cwd `arch/x86_32/`) → exit 0 (shell→usrlib chain, §11). `cargo xtask apps` → exit 0 (usrlib compiles for `rosx-user.json`).
- **Commit:** `Add UserDataHandle and bulk data conveniences to usrlib`

### Step 6 — `bulk_echo` service, `bulk_client`, boot wiring, QEMU integration test (spec increment 6, R12)

**Files (new):** `apps/bulk_echo/{Cargo.toml,build.rs,linker.ld,src/main.rs}`, `apps/bulk_client/{same}`, `tests-integration/tests/ipc_bulk.rs`. **Modified:** root `Cargo.toml`, `xtask/src/main.rs`, `arch/x86_64/build.rs`, `arch/x86_64/src/main.rs`.

- **New apps**, both cloned from the `random_gen_server` skeleton (`Cargo.toml` `[[bin]] bench=false test=false`, deps `system`+`usrlib`; `build.rs` = the 5-line `-T linker.ld` + `--pie` emitter; `linker.ld` copied verbatim; `main.rs` `#![no_std] #![no_main]`, `SyscallAllocator`, `_start`, own `#[panic_handler]` — all per `apps/random_gen_server/src/main.rs`). Add both to root `Cargo.toml` workspace `members` (after `apps/random_gen_server`, `:11`).
- **`bulk_echo` (server):** prints `[IPC Server] BulkEcho`; binds `"BULKECHO"`; loop `ipc_accept_message` → `wait_for_message` → `receive_data` to end → checksum = wrapping `u32` sum of every received byte → `println!("BULK Echo: {} {}", count, checksum)` → reply message back on `received.connection()` carrying the count via `ipc_send_value` (the client verifies counts from the reply; checksums from the printed lines). Reader posture: never calls `ipc_data_read`… **correction:** it reads via `receive_data` (that *is* `ipc_data_read`) and **releases nothing** (it is the reader — R3). Panic handler prints `[IPC Server] BulkEcho - Panic!`.
- **`bulk_client`:** loop `{ Syscall::ipc_connect("BULKECHO") }` until `Ok` (boot-order independence, BR-8). Deterministic byte generator shared with the test **by literal**: object *i* payload byte *j* = `(j * 7 + i) as u8`, lengths `[32768, 32768, 8192]` (total 73728 ≥ 64 KiB across **multiple** DataObjects, spec bar). Sequence, all markers exact (`|` = space):
  1. Oversized write: `ipc_data_write` with `65537` bytes → expect `Err(DataError::TooLarge)` → print `BULK TooLarge rejected` (else `BULK Mismatch toolarge`).
  2. Budget probe: write 64 KiB objects until one returns `Err(DataError::BudgetExceeded)` (the 5th write: 256 KiB outstanding ⇒ `+64 KiB > budget` fails; BR-11 math) → print `BULK Budget rejected`; release the 4 held handles (Vec drop = auto-release via BR-7 Drop — the ergonomics proof); write one more 64 KiB → `Ok` → print `BULK Refund ok`; release it.
  3. Verification round (per object): `send_data(conn, obj)` keeping ownership in a `Vec<UserDataHandle>` (no clones); wait for the reply (`ipc_receive_value`); compare echoed count; after all three, compare the summed bytes and print `BULK Verified: 73728` (or `BULK Mismatch verify`); release handles in order (explicit `release()` on the first object — the double-release guard of BR-7 is then exercised by the Vec drop).
  4. Disconnect, then print `BULK Done`, then `loop { Syscall::sleep(1000); }` (one-shot output, quiet task).
  Forbidden-by-construction failure lines (asserted absent): `BULK Mismatch`, `BULK Connection failed`, `[IPC Server] BulkEcho - Panic!`.
- **Wiring:**
  - `xtask/src/main.rs:5`: `const USER_APPS: [&str; 5]` → `[&str; 7]` with `"bulk_echo", "bulk_client"` appended (fixed-length array bump).
  - `arch/x86_64/build.rs`: mirror lines 4-6 (rerun-if-changed ×3) for **both** `apps/bulk_echo` and `apps/bulk_client`, and duplicate the existence-panic block of `:8-11` for `../../target/rosx-user/release/bulk_echo` and `.../bulk_client` — same commit as the embedding, or every kernel build from a clean target dies.
  - `arch/x86_64/src/main.rs` after `:75` (random_gen_server embed+schedule): two `include_bytes!("../../../target/rosx-user/release/{bulk_echo,bulk_client}")` statics + `let _ = kernel.schedule(new_elf_task(...));` each. **Do not touch `apps/shell/`** (BR-8 i686 trap).
- **New integration test** `tests-integration/tests/ipc_bulk.rs` — template `tests/ipc_random.rs`: `QemuSession::spawn()`; **first action**: `let mark = session.mark();` (the exchange is boot-time); `expect_output("[IPC Server] BulkEcho", READY_TIMEOUT=60s)`; `expect_output("rose>", READY_TIMEOUT)`; `wait_until_since(mark, |delta| delta.contains("BULK Done"), SWEEP_TIMEOUT=180s)` (panic with tail on timeout, per template); then `content = session.content_since(mark)` and assert (machine-countable):
  - `count_prefixes(content, "BULK Echo: ") == 3`; parsed triples `(bytes, checksum)` equal the host-recomputed expected set from the same `(j * 7 + i)` generator and lengths `[32768, 32768, 8192]` (checksum = wrapping u32 sum — pure host arithmetic, no trust in the guest).
  - exactly one line each containing `BULK TooLarge rejected`, `BULK Budget rejected`, `BULK Refund ok`; `BULK Verified: 73728` exactly once; `BULK Done` exactly once.
  - none of the forbidden strings above appears (FAILURE_STRINGS list, template `:13-21` style); reuse `count_prefixes`/`tail_chars`/diagnostics pattern.
- **Why:** R12's "client writes ≥ 64 KiB across multiple DataObjects + oversized write exercising `IpcDataTooLarge`/`IpcDataBudgetExceeded`" plus the Drop-refund proof, all asserted on the debugcon text stream exactly like `ipc_random.rs`.
- **Verify (local; CI never runs tests-integration — Context Map §8):** `cargo xtask build` → exit 0 (apps + kernel with both embeds + disk image). `cargo xtask test` → exit 0 with the suite now reporting the original 7 QEMU tests **plus** `ipc_bulk` green (baseline Gate 2's QEMU evidence proves the harness runs non-vacuously).
- **Commit:** `Add bulk_echo service, bulk client and QEMU integration test` (single commit keeps the build.rs ↔ USER_APPS ↔ embed set consistent; do not split — any subset breaks a clean-target build).

### Step 7 — docs + full verification pass (spec increment 7, R13/R14)

**Files:** `AGENTS.md` only.

- **Modify** `AGENTS.md:33` — current verbatim: `│    elf/ ipc/ (mailbox) future/ syscall/ keyboard/ task*  cpu (HAL trait)  kconfig` → replace `ipc/ (mailbox)` with `ipc/ (mailbox, data)`.
- **Modify** `AGENTS.md` "Kernel Subsystems (current invariants)" list: add one bullet after the `future/` + `ipc/` bullet stating the invariant verbatim in spirit: `ipc/ data family — messages signal; data rides the connection out-of-band via write/read/release tokens; the kernel never interprets message payloads.` Keep it one line (R14).
- **Verify:** the full acceptance section below.
- **Commit:** `Document IPC data family in AGENTS.md`

---

## 3. Final acceptance (all on the final tree; R13 verbatim)

From repo root `/home/ghisi/git/rosx`:

1. `cargo test -p kernel -- --test-threads=1` → exit 0; ≥ 224 baseline kernel tests plus Step-2/Step-3 additions, 0 failed.
2. `cargo xtask test` → exit 0; 260 baseline host tests plus additions, and QEMU suite = baseline 7 + `ipc_bulk` = 8, 0 failed (this gate **is** the R12 QEMU run — CI does not run it).
3. `cargo xtask build` → exit 0 through "create x86_64 disk image".

From `arch/x86_32/`:

4. `cargo build -p rosx-x86` → exit 0; `rosx-x86` warnings must not exceed the baseline 1 (`unused_unsafe`) — Δ-new = 0.

**Warning arbitration (Δ-new = 0 vs §0 fingerprint):** count both `warning:` and `warning[EXXXX]:` lines per cargo target summary; attribute by file location — a diagnostic located only in unchanged files is baseline and never a violation; totals corroborate but the location diff convicts; decreases at sites this work legitimately deletes are expected (this plan deletes no warning sites — expect flat multiset). A gate failure requires a warning whose location is a file this branch touched.

**Machine-countable QEMU evidence:** the `ipc_bulk` assertion set (Step 6) — 3 echo triples byte-equal to host-recomputed checksums, singletons of `BULK TooLarge rejected` / `BULK Budget rejected` / `BULK Refund ok` / `BULK Verified: 73728` / `BULK Done`, zero forbidden strings.

---

## 4. Traceability R1–R14 → steps

| Req | Covered by | How provable |
|---|---|---|
| R1 Message untouched | never touched; Step 1 grows only its test lists | `system/src/ipc.rs` layout tests (`:32-42`) green in gates 1/2 |
| R2 complementary families | Steps 1,4 (additive arms; `Err(_) => 0` preserved) | gates 1–4 + existing syscall behavior green |
| R3 lifecycle | Step 3 (`write/read/release` rules, `evict` single path) | data_manager tests 1–12, 15 |
| R4 authorization | Step 3 (`write_data`/`read_data` per BR-5) | tests 16–19 |
| R5 budget | Step 3 (TooLarge/BudgetExceeded/refund/per-task) | tests 5–8; QEMU probe markers |
| R6 disconnect GC | Step 3 (`gc_connection` on `disconnect` success path) | tests 13–14, 16 |
| R7 error codes | Step 1 (codes+codec), Step 3 (`DataError` From projections) | codec sweeps (13-wide arrays), errors.rs mapping test |
| R8 handle pattern | Step 2 (fifth newtype), Step 5 (opaque `UserDataHandle`) | Step-2 roundtrip/sweep/garbage tests; usrlib hex-only Debug test |
| R9 usrlib ergonomics | Step 5 (wrappers, `UserDataHandle::release`+`Drop`, `send_data`/`receive_data`) | host tests (None-guarded) + QEMU refund marker |
| R10 treaty discipline | Step 1 adds numbers/codes only; no Data types in `system` | file list of Step 1 |
| R11 unit tests | Steps 2–3 (19 tests enumerated) | gate 1 test count delta, names above |
| R12 QEMU proof | Step 6 (`ipc_bulk.rs` + boot-scheduled pair) | gate 2 QEMU section; assertion set §3 |
| R13 gates green | Steps 1–7 per-step gates + §3 | four commands verbatim above |
| R14 docs | Step 7 (`AGENTS.md:33` + invariant bullet) | diff |

## 5. Residual risks (coder + orchestrator awareness)

1. **Host `raw_syscall` hazard (BR-7):** any future usrlib test that drops a live `UserDataHandle` fires a real Linux syscall (#26 `msync`) with garbage args. Mitigated by the `Option`-gated Drop + the Step-5 test rule; enforce in review of later diffs.
2. **`app_lifecycle` watchdog vs boot-time bulk output:** the boot-time client prints during every integration session (prefix-isolated `BULK `, one-shot, then sleeps). Baseline showed the suite tolerant of additive output; if `app_lifecycle` ever flakes on it, remediation is delaying the client's first print by N sleeps, not moving it into the shell (BR-8 trap still holds).
3. **Slot-recycle parity assertion (test 12):** the exact index/generation parity copies the `ipc_manager.rs:242-264` template; if the arena's round-robin order shifts the count, the binding assertion is stale-token `read` → `NotFound` (parity assertions are parity-only).
4. **Gate 2 runtime:** baseline ran 7 QEMU tests in the 900 s wrapper with headroom; `ipc_bulk` adds roughly one boot + one 180 s-capped exchange. If the wrapper ever fires, run gates separately (never drop the QEMU suite from acceptance).
5. **`cargo xtask test --skip-integration` / `cargo xtask apps` as step gates:** flag existence verified in `xtask/src/main.rs` and CI usage per Context Map §8, but only the plain `cargo xtask test` form has baseline-run green-verbatim evidence; treat the two variants as orchestrator-observed first use (any first-use failure is a plan gate issue, not a code issue).
6. **x86_32 runtime:** new syscalls compile and are reachable via `int 0x80` there, but no suite exercises x86_32 IPC at runtime (Context Map §11) — acceptance is compile-only, as the spec's own gates state.
7. **Budget-probe heap pressure in QEMU:** worst case ~5 × 64 KiB user buffers + 4 held kernel objects ≈ 512 KiB across the two tasks — bounded by design (R5); monitor the debugcon tail on first QEMU run.
