# Kernel-Owned Handle Domain Model — Implementation Plan

> **Date:** 2026-10-11
> **Branch:** `kernel-owned-handle` (= `7680e54`, docs-only ahead of `main` @ `7171686`)
> **Spec:** `plans/kernel-owned-handle.md` (LOCKED) — R1–R15, OD1/OD2 resolved, Q2a adopted.
> **Context Map:** `.opencode/artifacts/kernel-owned-handle-context-map.md` — corrections C1–C10 are binding inputs; every one is folded into the steps below.
> **Executor:** coder. This plan defines contracts; the coder implements them. Where a prose contract and a code fragment conflict, the contract and the acceptance gates win.

---

## 0. Locked decisions (restated, not open)

- **OD1 — user-facing error surface: mirror enums.** Per-domain mirror error enums live in `usrlib` (`IpcSendError`, `IpcReceiveError`, `IpcConnectionError`, `IpcBindingError`), mapped from `system::error::ErrorCode` at the user edge. `ErrorCode` remains the wire vocabulary underneath, never the app-facing type. Precedent: `usrlib::ipc::ReceiveValueError` (`usrlib/src/ipc.rs:11-27`).
- **OD2 — user handle kinds: mirror the kernel types.** Three opaque user-side handle kinds — future / connection / binding — so the user API refuses a binding where a connection belongs at compile time.
- **R13 governance of OD1/OD2:** token carriers, not validators. User handle types wrap the opaque packed wire value only — never decompose, never re-encode, never re-check a kernel invariant, duplicate no kernel layout constant or bit arithmetic. The boundary codec call into `system` is the sole contact point. Compliance by type shape + at least one negative compile check (Step 18).
- **Q2a uapi model:** both worlds depend on the shared treaty crate `system`; the no-shared-crate model is rejected (spec §5).
- **R8 WIRE FREEZE (overrides everything):** packed 32-bit layout (bit 31 = `ERROR_BIT`, generation bits 16–30, index bits 0–15), `ErrorCode` numbering `0x0101`–`0x0108` + `Unknown = 0x7FFF_FFFF`, `Message` 64 bytes / `conn: u32` @ offset 0 / align 4, all syscall numbering and semantics — bit-identical before and after. Any wire diff is a defect.
- **Translation placement law:** translation is implemented at the two boundary sites only — kernel encodes domain→wire (`kernel/src/syscall.rs`); usrlib decodes wire→user-view. `system` holds only what both sides legitimately share: numbers/layouts and pure raw-`usize` codec arithmetic that names no domain type.
- **Constraints:** `no_std` everywhere; `system` gains no dependencies; kernel deps after refactor: `system` + `lazy_static` only (net-neutral vs R11); "no comments, ever" (assembly excepted); commit subjects bare imperative; AGENTS.md "work incrementally" — one commit at a time, gate green before the next.

## 0.1 Baseline facts (sourced, not re-derived)

Pre-change test totals — `collections` 21, `system` 12, `kernel` 182, `shell` 5, `cargo xtask test --skip-integration` total 220 (delegation-confirmed; Context Map C6 confirms the sum). QEMU tests: `boot_and_shell` 4, `ipc_random` 1 (excluded from the 220). Prior warning fingerprint (PR #57 tree, run-20261010-205728): kernel lib 65, rosx bin 26, shell 5, rosx-x86 1, tests-integration lib 1 — **reference only; this refactor moves code, so the verifier must re-establish the baseline on the branch-point commit (BR-15). No final post-refactor test count is asserted anywhere in this plan (R12): all suite totals are re-baselined by the verifier baseline pass.**

**RE-ESTABLISHED BASELINE (verifier pass 2026-10-11 @ `7680e54`, logs `/tmp/opencode/run2-baseline/`):** gates G1 21 / G2 12 / G3 182 / G4 220 / G5 / G6 / G7a(release) / G7b / G8 4 (20.95s) / G9 1 (20.56s) — all green. Warning fingerprint Δ vs prior-run reference: **none** (kernel lib 65, rosx bin 26, shell lib 5, rosx-x86 1, tests-integration lib 1); code-bearing `warning[E0133]` line counts g5 47 / g6 40 / g7b 40 — E0133 is the only warning code anywhere. Two reference-absent items captured at baseline and hereby part of it: `rosx` manifest warning ×1 (`unused dependency dummy`, g5) and the `core` future-incompat notice lines (g5 ×2, g6 ×1). Absence-gate error text pinned: `error: package ID specification \`<name>\` did not match any packages` (exit 101). Vacuous-gate hazard confirmed: a positional test-name argument (e.g. `cargo test -p collections some-name`) exits 0 selecting zero tests — never append positional filters to gate commands.

## 0.2 Blessed gate shorthand (Context Map §7, `/tmp/opencode/run_gates.sh`)

| ID | Command |
|----|---------|
| G1 | `cargo test -p collections` (pre-removal only) |
| G2 | `cargo test -p system` |
| G3 | `cargo test -p kernel -- --test-threads=1` |
| G4 | `cargo xtask test --skip-integration` |
| G5 | `cargo xtask build` |
| G6 | `cargo build -p kernel --features lazy_static/spin_no_std --target arch/x86_64/rosx.json -Zbuild-std=core,alloc,compiler_builtins -Zbuild-std-features=compiler-builtins-mem -Zjson-target-spec` — **the `--features lazy_static/spin_no_std` is load-bearing** (bare form without it is structurally broken pre-existing; do not "fix" it) |
| G7a | `cargo build --release -p hello_elf -p random_gen_server -p snake -p tetris -p conway --target arch/x86_32/rosx-i686-user.json -Zbuild-std=core,alloc,compiler_builtins -Zbuild-std-features=compiler-builtins-mem -Zjson-target-spec` (from root) — **BASELINE PIN: `--release` is load-bearing.** The debug form passes on its own but starves G7b: `apps/shell/build.rs:10-22` demands `target/rosx-i686-user/release/{hello_elf,snake,tetris,conway}`; verbatim debug G7a → G7b exits 101 on a clean tree (verifier baseline 2026-10-11) |
| G7b | `cd arch/x86_32 && cargo build -p rosx-x86` — **must run after G7a(RELEASE)** (load-bearing order; from root it compiles for host and dies) |
| G8 | `cargo test -p tests-integration --test boot_and_shell` |
| G9 | `cargo test -p tests-integration --test ipc_random` |

G8/G9 are scriptable — **never label them manual** (C10). CI runs only {`cargo xtask test --skip-integration`, `cargo xtask build`} (+ push-only release); everything else is local gate-chain only.

---

## 1. Binding resolutions (numbered; the coder follows these verbatim)

Spec-vs-tree conflicts and ambiguities, resolved here as the plan's law:

- **BR-1 (apps scope amended).** Spec §2/§4 say apps change only in the shell debug print. Tree facts contradict: R7 changes `Syscall::exec`/`load`/`ipc_receive_message`/`ipc_accept_message` signatures consumed by `apps/shell/src/shell.rs:107-123` (5 sites), `apps/test_suite/src/app.rs:8-24` (5 sites), and `Message::conn()`'s type leak is consumed by `apps/random_gen_server/src/main.rs:42` (`ipc_send_value(envelope.conn(), value)`). Resolution: those call-site adaptations are **in scope** (Step 14); they are mechanical signature/edge adaptations with zero functional change. No other app changes.
- **BR-2 (transient arena duplication is wire-atomic).** The collections crate cannot be deleted until both `system` and `kernel` are off it, and `system` cannot be stripped until kernel+usrlib stop consuming its domain surface. Commits therefore carry a bounded duplication (arena file: collections crate ↔ kernel copy; enums: system ↔ kernel; system old + new Message fns) where **each consumer world is closed over exactly one copy** — no commit boundary mixes old source layout with a half-moved dependency edge. Duplications are removed by C6; nothing references both copies of any item.
- **BR-3 (`ERROR_BIT` home and kernel layout expression, R3).** `ERROR_BIT` becomes *defined* (not re-exported) in `system/src/error.rs`, replacing line 4 (`pub use collections::generational_arena::ERROR_BIT;`) with `pub const ERROR_BIT: usize = 1 << 31;` (value-identical to `collections/src/generational_arena.rs:4`). `system` holds **no** INDEX/GENERATION masks (usrlib may never hold them per R13; system needs none: `ErrorCode::from_code` already masks via `raw & (ERROR_BIT - 1)`, and classification is a single predicate — see §3.2). The kernel handle layout is expressed against `system::error::ERROR_BIT` in `kernel/src/handle.rs` using the current expressions verbatim from `collections/src/generational_arena.rs:13-15`: `INDEX_BITS: usize = 16`, `INDEX_MASK: usize = (1 << INDEX_BITS) - 1`, `GENERATION_MASK: usize = (ERROR_BIT - 1) >> INDEX_BITS`. Single source of the numbers; enforcement (debug asserts, wrap, cycle tests) is kernel policy (R10).
- **BR-4 (sentinel, R7 kernel-side choice).** The kernel **keeps emitting `u64::MAX as usize`** at `kernel/src/syscall.rs:29` and `:83` (verbatim), so the failure-path wire word is bit-identical (R8). The user side decodes it to `None` for free: `u64::MAX & ERROR_BIT ≠ 0` ⇒ not a token ⇒ `Option::None` via the codec. No dedicated new error code is minted.
- **BR-5 (non-canonical inbound words, R10).** Every kernel decode of a user-supplied word goes through `from_packed`, which validates canonicality exactly as `is_handle` does today (`collections/src/generational_arena.rs:41-43`, body `raw < ERROR_BIT` — relocated verbatim as `system::error::is_token`). Failure policy: handle-returning syscalls return the wire error word `ErrorCode::Unknown.to_reg()`; void/no-error-channel syscalls return `0`. Documented behavior delta (R10-sanctioned, outside freeze because no legitimate kernel-issued word is affected): today a crafted `conn` word with bit 31 set is *masked* by `Handle::unpack` and could alias a live connection (`ipc_manager.rs:121`); after the change it is rejected `IpcSendError::ConnectionNotFound`. `ipc_random` exercises only legitimate words.
- **BR-6 (mirror mapping table, R13/R8).** The usrlib mirror enums keep **today's exact variant sets** and reproduce **today's exact match arms** — the implicit fallback is the catch-all category (an added `Unknown` variant would change what apps can observe):
  - connection: `IpcConnectionCannotBeEstablished` ⇒ `ConnectionCannotBeEstablished`, `_` ⇒ `ServerNotFound` (system/src/ipc.rs:45-50)
  - binding: `_` ⇒ `AlreadyBound` (system/src/ipc.rs:63-67)
  - send: `IpcSendConnectionCongested` ⇒ `ConnectionCongested`, `_` ⇒ `ConnectionNotFound` (system/src/ipc.rs:81-86)
  - receive: `IpcReceiveNoMessagesAvailable` ⇒ `NoMessagesAvailable`, `IpcMailboxNotAvailable` ⇒ `MailboxNotAvailable`, `_` ⇒ `ConnectionNotFound` (system/src/ipc.rs:110-116)
  `Display` strings preserved verbatim ("Connection not found", "Connection congested", "No messages available", "Mailbox not available") — the `ipc_random` FAILURE_STRINGS sweep and shell prints depend on the surrounding app strings.
- **BR-7 (doc-comment on `Future::complete`).** `system/src/future.rs:10-12` carries a doc comment (tolerated in-tree per Context Map §9). The relocation is content-preserving: move the comment verbatim to the kernel trait. Comment-policy cleanup is out of scope for this PR.
- **BR-8 (newtype visibility, R4).** All four kernel newtypes are `pub` in the `kernel` crate — required by existing `pub` interfaces (`Kernel::schedule(...) -> Result<FutureHandle, ()>` at `kernel/src/kernel.rs:180`, `pub trait ForCompletingExpiredTimers` at `kernel/src/ports/driven.rs:18`). Fields stay private; decomposition accessors (`index()`, `generation()`, `new()`, `pack()`, `from_packed()`) are `pub(crate)` — none reachable outside the `kernel` crate (R4). If the `pub(crate)`→`pub` widening of `TaskHandle` (today `pub(crate) type` at `kernel/src/task.rs:10`) removes any private-interface warning, that decrease is attributed at the warning gate by multiset diff, never hidden (decreases are fine; increases/new codes are not).
- **BR-9 (no fifth newtype).** `MailboxHandle` (`kernel/src/ipc/mailbox_manager.rs:21`) remains `pub(crate) type MailboxHandle = Handle` (arena identity). It never crosses the wire; R4's count is exactly four newtypes. Do not "promote" it.
- **BR-10 (R9 QEMU gate wording).** The 259-sweep shell loop over a 256-slot connection arena recycles slots with a generation bump (0→1); it does not perform a literal 32768-wrap (that law stays proven by the relocated unit cycle test, §5). The QEMU assertion pair is therefore: **after the first connection's slot has been recycled, the stale token is rejected and a fresh token works** — proved by shell-minted deterministic lines matched by the test (Step 14/15), not by numeric generation arithmetic parsed from handle internals.
- **BR-11 (naming hygiene vs the R2 gate).** The system classify predicate is named `is_token` (the lowercase `is_handle` would pass a case-sensitive `\bHandle\b` gate but reads like naming the type; avoid the argument). The R2 gate is case-sensitive `grep -E` as written in §7.
- **BR-12 (workspace counts).** Member removal: workspace members 16 → 15, default-members 8 → 7 (Context Map §7 clarifies the spec's "6 → 5 crates" as the lib-style count).
- **BR-13 (test totals).** No step or gate in this plan asserts a final test count. Every suite gate is "zero failures; totals as re-established by the verifier baseline".
- **BR-14 (no manual runtime gates).** G8/G9 are scriptable QEMU sessions (`QemuSession::spawn` / `expect_output` / `mark`); they are scripted gates. The only manual label survives for the warning-baseline bookkeeping (§7.4), for which no repo tooling exists (Context Map §8).
- **BR-15 (warning baseline re-establishment).** The Δ-new=0 gate compares against a fingerprint **re-established by the verifier at the branch-point commit** (before Step 1), using the procedure in §7.4. The run-20261010 numbers (kernel lib 65 / rosx bin 26 / shell 5 / rosx-x86 1 / tests-integration lib 1) describe the PR #57 tree and are not gate values for this PR; the plan document's self-attestation is not evidence of any command's green status.

---

## 2. Commit lattice (wire-atomic boundaries)

Dependency arrows: `collections ← system ← {kernel, usrlib ← apps}`; `collections ← kernel`; `collections ← usrlib` (unused, C4-of-Context-Map). Dissolution order is strictly consumer-first, subtraction-last. Eight commits; each keeps its listed gates green; QEMU gates G8/G9 pass at every boundary (shell print change and `ipc_random` re-architecture land in the **same** commit — C5 — so the harness never sees a parse break).

| Commit | Subject (bare imperative) | Content | Gates |
|---|---|---|---|
| C1 | Drop unused collections dependency from usrlib | `usrlib/Cargo.toml` one-liner (C4: already unused in source) | G4 |
| C2 | Relocate generational arena into kernel | arena file copied to `kernel/src/collections/`; 11 kernel consumers (C1-list); kernel loses its `collections` edge | G3, G6 |
| C3 | Define wire ABI constants and raw token codec in system | `ERROR_BIT` defined directly; `is_token`; `Message::new_packed`/`conn_packed`; additive only | G2 |
| C4 | Introduce kernel handle domain, Future trait, and IPC error vocabulary | `kernel/src/handle.rs` (Handle + 4 newtypes + layout law); kernel `Future` trait; kernel `IpcMessageFuture`; kernel IPC error enums + `From → ErrorCode`; all kernel consumers; kernel edge in `syscall.rs` | G3, G4, G6, G8, G9 |
| C5 | Move user edge to opaque handles and mirrored errors | `usrlib/src/handle.rs`; mirror enums; `Syscall` signature flip; apps (shell, test_suite, random_gen_server); `ipc_random` re-architecture (both sides) | G4, G5, G8, G9 |
| C6 | Reduce system to the ABI treaty and dissolve the collections crate | system subtraction (R2 gate goes green here); `collections/` deleted; workspace 16→15 | ALL G1..G9 (G1 retired), grep gates §7.3 |
| C7 | Add negative compile check for user handle kinds | `checks/compile_fail.sh` | script (unverified → baseline pass) |
| C8 | Document kernel-owned handle architecture | README + AGENTS (R14/R15) | doc greps §7.5 |

R2's greps may contain matches until C6; every **interim** commit verifies instead that no consumer references a copy it no longer owns (per-step greps below).

---

## 3. Interfaces and signatures (contracts; coder implements)

### 3.1 `kernel::collections::arena` (new, `kernel/src/collections/mod.rs` + `arena.rs`)

- C2: verbatim relocation of `collections/src/generational_arena.rs` as `arena` (items `GenerationalArena<T, const S: usize>`, `Error { NotFound, OutOfMemory }`, `Handle`); its 21 tests ride along (C4 relocates the layout/policy subset — §5).
- C4 end-state: `arena` keeps `GenerationalArena` + `Error` + slot storage; `Handle` moves to `kernel/src/handle.rs` and comes back via `use`. `Error` stays here (kernel-side domain vocabulary per the delegation; consumers `kernel.rs:22`, `future.rs:6` re-point). No new generic parameters — the arena stays concrete (spec §4: no re-generalization).
- Gates that must keep passing here: the 3 arena `should_panic` tests' assertion strings verbatim: `"S must be greater than zero"`, `"arena exceeds 16-bit index space"`; handle `"generation outside 15-bit handle space"`.

### 3.2 `system` final surface (treaty only)

`system/src/error.rs`:
- `pub const ERROR_BIT: usize = 1 << 31;` (BR-3)
- `enum ErrorCode` **unchanged** (numbers frozen by R8): `IpcServerNotFound = 0x0101` … `IpcMailboxNotAvailable = 0x0108`, `Unknown = 0x7FFF_FFFF`; `to_reg` / `from_code` unchanged.
- `pub const fn is_token(raw: usize) -> bool { raw < ERROR_BIT }` (body verbatim from `is_handle`).
- **Deleted:** `use collections::...`, the `ERROR_BIT` re-export, `mod private`, `trait ErrorCodeOf` + all four impls (R6), `into_reg`, `from_reg`.
- Kernel-side replacement of `ErrorCodeOf`: four `impl From<kernelIpcError> for ErrorCode` projections in `kernel/src/ipc/errors.rs`, mapping verbatim from the deleted table (`system/src/error.rs:48-92`). Kernel edge: `Err(e) => ErrorCode::from(e).to_reg()`.

`system/src/ipc.rs`:
- Kept: `MESSAGE_BYTES: usize = 64`, `MESSAGE_PAYLOAD_BYTES: usize = 60`, `Message` (`#[repr(C)]`, `conn: u32` private @ 0, `data: [u8; 60]`, align 4 — R8), `EMPTY`, `data()`.
- Final ctors/accessors (renamed in C6 from the C3-additive packed pair, BR-2): `pub fn new(conn: u32, data: &[u8; MESSAGE_PAYLOAD_BYTES]) -> Self` and `pub const fn conn(&self) -> u32` — **raw u32 in and out, names no domain type** (kills the `ipc.rs:25-27` leak).
- **Deleted:** `use collections::...`, `pub type IpcConnectionHandle` / `pub type IpcBindingHandle` (ipc.rs:5-6), the four IPC error enums with their `to_reg`/`from_reg`/`Display` (kernel + usrlib take their copies).
- `system/src/future.rs`: **deleted file** + `pub mod future;` removed from `lib.rs` (trait → kernel; `FutureHandle` alias → kernel newtype). `lib.rs:3` `extern crate collections;` deleted (C2-of-Context-Map: the `extern crate` form a `use`-only grep misses — the §7.2 gate pattern covers it).

`system/Cargo.toml`: `[dependencies] collections = { path = "../collections" }` (line 9) deleted. `system` gains no dependencies (R11).

### 3.3 `kernel::handle` (new, `kernel/src/handle.rs`; `pub mod handle;` added to `kernel/src/lib.rs`)

Underlying identity (moved from arena in C4):
```
pub(crate) struct Handle { index: u16, generation: u16 }   // fields pub(crate); derives Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Debug (Debug prints {index, generation} — R5 legal kernel-side aid)
pub(crate) const INDEX_BITS: usize = 16;
pub(crate) const INDEX_MASK: usize = (1 << INDEX_BITS) - 1;
pub(crate) const GENERATION_MASK: usize = (system::error::ERROR_BIT - 1) >> INDEX_BITS;   // BR-3
pub(crate) const fn next_generation(current: u16) -> u16    // body verbatim: (current + 1) & (GENERATION_MASK as u16)
```
Four newtypes (`TaskHandle`, `FutureHandle`, `IpcConnectionHandle`, `IpcBindingHandle`), each **`pub`, identical shape**, private single field `Handle`:
- derives `Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Debug` (BTreeMap keys need `Ord`: `waiters: BTreeMap<FutureHandle, Vec<TaskHandle>>` `kernel/src/future.rs:83`, `registry: BTreeMap<String, IpcBindingHandle>` `ipc_manager.rs:49`, `yield_info: BTreeMap<TaskHandle, YieldInfo>` `mlfq_strategy.rs:19`).
- `pub(crate) const fn new(index: u16, generation: u16) -> Self` — carries today's `debug_assert!` with message verbatim `"generation outside 15-bit handle space"` (R10; const so `ipc_manager.rs:166-167` `const CLIENT_TASK: Handle = Handle { index: 1, generation: 1 };` becomes `const CLIENT_TASK: TaskHandle = TaskHandle::new(1, 1);`).
- `pub(crate) fn pack(self) -> usize` — same bit arithmetic as `collections/src/generational_arena.rs:25-32`, debug assert on the 15-bit domain preserved verbatim.
- `pub(crate) fn from_packed(raw: usize) -> Option<Self>` — `system::error::is_token(raw)` guard (BR-5 canonicality), then split via `INDEX_MASK` / `INDEX_BITS` / `GENERATION_MASK`.
- `pub(crate) fn index(self) -> u16`, `pub(crate) fn generation(self) -> u16` — **no external reachability (R4)**.
- `pub(crate) From<Handle> for Newtype` and `pub(crate) From<Newtype> for Handle` (arena interop, kernel-internal only).
- No `unpack`, no public index/generation path, no wire constants re-declared anywhere else.

Domain re-exports (churn control): `kernel/src/task.rs` keeps `pub use crate::handle::TaskHandle;` so the ~15 `crate::task::TaskHandle` sites (ports/driven.rs, kernel.rs, future.rs, ipc_manager.rs, …) are untouched; every `use system::future::{Future, FutureHandle}` / `use system::ipc::{IpcConnectionHandle, IpcBindingHandle}` site switches to `crate::future::Future` / `crate::handle::*`. **Full consumer set (C1 + `system::future` sweep, all files):** `kernel/src/lib.rs:5` (extern crate, removed in C2), `ipc/ipc_manager.rs:4-6`, `ipc/mailbox_manager.rs:4-6` (incl. full-path `dyn system::future::Future` / `system::future::FutureHandle` in `NoopNotifier` :12-16), `task.rs:1,8`, `future.rs:5-6`, `task_manager.rs:1,5` (+ test :101), `kernel.rs:22-23` (+ full path `alloc::vec::Vec<system::future::FutureHandle>` at :63), `syscall.rs:6-9`, `kernel_services.rs:13-14`, `keyboard.rs:2`, `ports/driven.rs:3-4`, `scheduler/timer.rs:3` (+ test :39), `scheduler/fifo_strategy.rs:1` (test `make_handle` :57-58), `scheduler/mlfq_strategy.rs:2` (test `make_handle` :101-102), `scheduler/scheduler.rs:219,224` (test), `ipc/mailbox.rs:2` (Message only — unchanged).

### 3.4 `kernel::future` and `kernel::ipc` additions

- `Future` trait moves verbatim (incl. BR-7 doc comment) from `system/src/future.rs:7-20` into `kernel/src/future.rs`. Impls updated: `TimeFuture`, `TaskCompletionFuture`, `DummyFuture` (tests), `KeyboardFuture` (`keyboard.rs:42`), `IpcMessageFuture` (new home below).
- `kernel/src/ipc/message_future.rs`: `IpcMessageFuture` moved verbatim from `system/src/ipc.rs:133-179` (fields `message: Option<Message>`, `error: Option<kernel IpcReceiveError>`; `new/with_message/with_error/complete/result`; `impl Future` with `as_any/as_any_mut/into_any`). Registered behind `Box<dyn Future + Send + Sync>` in `FutureRegistry`; **downcast identity changes crate-internal path only** — see blast radius §6.3.
- `kernel/src/ipc/mod.rs` gains `mod message_future;` and `mod errors;`.
- `kernel/src/ipc/errors.rs`: the four kernel domain error enums — variants verbatim from `system/src/ipc.rs:34-127` minus `to_reg`/`from_reg` (kernel only encodes); needed derives: `Debug, Copy, Clone, PartialEq, Eq` (+ `Clone` retained where tests clone, cf. `system/src/error.rs:185`); the four `impl From<…> for system::error::ErrorCode` (R6, verbatim mapping of `system/src/error.rs:48-92`).

### 3.5 Kernel edge — `kernel/src/syscall.rs` (domain→wire, the sole kernel encode site)

- `Exec` (:25-31) / `LoadElf` (:78-85): match arms unchanged; failure keeps literal `u64::MAX as usize` (BR-4).
- `WaitFuture` (:49-61): `FutureHandle::from_packed(arg1 as usize)` — `None` ⇒ return `0`; downcast to `crate::ipc::message_future::IpcMessageFuture` (:53); `Err(e) => ErrorCode::from(e).to_reg()` (:57 replacement).
- `IsFutureCompleted` (:63): `from_packed`, `None` ⇒ `0`.
- `IpcConnect` (:88) / `IpcBind` (:97): replace `into_reg(...)` with two small local helpers (or inline matches): `Ok(h) => h.pack(), Err(e) => ErrorCode::from(e).to_reg()`.
- `IpcDisconnect` (:91), `IpcReceiveMessage` (:106), `IpcAcceptMessage` (:107): `from_packed`; on malformed input the handle-returning ones emit `ErrorCode::Unknown.to_reg()`, void ones `0` (BR-5).
- `IpcSendMessage` (:99-105): envelope decode moves into `IpcManager::send_message` (see :120-121 — `let connection = envelope.conn();` becomes `IpcConnectionHandle::from_packed(envelope.conn() as usize)` with `None` ⇒ `Err(IpcSendError::ConnectionNotFound)`); error arm `Err(e) => ErrorCode::from(e).to_reg()`.
- Kernel `Message` construction (tests, and any prod site): `Message::new_packed(h.pack() as u32, &data)` (C3 fn; renamed to `new` in C6). `IpcConnectionHandle::from_packed(... ) == Some(connection)` replaces `message.conn() == handle` comparisons in kernel tests.

### 3.6 `usrlib` user edge (wire→user-view, the sole user decode site)

`usrlib/src/handle.rs` (new; `pub mod handle;` added to `usrlib/src/lib.rs`):
- `pub struct UserFutureHandle { packed: u32 }`, `pub struct UserConnectionHandle { packed: u32 }`, `pub struct UserBindingHandle { packed: u32 }` — fields private, derives `Copy, Clone, PartialEq, Eq`.
- **R5:** custom `Debug` printing the packed value as hex only — contract: `{:08x}` lowercase zero-padded, no prefix; `Display` delegates to the same rendering (shell print, Step 14/15). Never a struct/field render.
- `pub(crate) fn packed(self) -> u32` — token forwarding for envelope construction inside `usrlib` only (not bit arithmetic; apps cannot reach it).
- `pub(crate) fn from_token(raw: usize) -> Option<Self>` — guards with `system::error::is_token(raw)` (the single codec contact point), then the single documented lossless cast `raw as u32` (lossless because `is_token ⇒ raw < 2^31`). No other construction path exists → a value of each kind is obtainable only through the codec return of the syscall that issues it, consumable only by APIs accepting that kind (type-shape compliance, R13).
- **Zero** bit operators and zero layout constants in `usrlib/src` (gate §7.3).

Mirror error enums (OD1) in `usrlib/src/ipc.rs` (or `usrlib/src/error.rs` — coder's choice, exported from `usrlib`): `IpcConnectionError`, `IpcBindingError`, `IpcSendError`, `IpcReceiveError` per BR-6, each `impl From<system::error::ErrorCode>` total with the catch-all fallback; `Display` on send/receive (and all four for uniformity) with today's strings. `ReceiveValueError` keeps its precedent shape and **gains `NoHandle`** with Display `"No handle returned"` (decode-`None` case; cannot occur on current wire — defensive, keeps R13 honesty).

`usrlib/src/syscall.rs` final signatures:
```
pub fn exec(entrypoint: usize) -> Option<UserFutureHandle>
pub fn load(elf: &'static [u8]) -> Option<UserFutureHandle>
pub fn wait_future(handle: UserFutureHandle)
pub fn wait_for_message(handle: UserFutureHandle) -> Result<ReceivedMessage, IpcReceiveError>
pub fn is_future_completed(handle: UserFutureHandle) -> bool
pub fn ipc_connect(service: &str) -> Result<UserConnectionHandle, IpcConnectionError>
pub fn ipc_disconnect(connection_handle: UserConnectionHandle)
pub fn ipc_bind(service: &str) -> Result<UserBindingHandle, IpcBindingError>
pub fn ipc_send(connection: UserConnectionHandle, data: &[u8; MESSAGE_PAYLOAD_BYTES]) -> Result<(), IpcSendError>
pub fn ipc_receive_message(connection_handle: UserConnectionHandle) -> Option<UserFutureHandle>
pub fn ipc_accept_message(binding_handle: UserBindingHandle) -> Option<UserFutureHandle>
```
Decode bodies are one pattern: call `arch::raw_syscall`, then `UserXHandle::from_token(raw)` / `system::error::is_token(raw)` + `ErrorCode::from_code(raw)` → mirror `From`. `wait_for_message` success condition is `is_token(raw)` (mirrors today's `!from_reg(raw).is_err()` bit-for-bit: today's `Ok` path accepted exactly `raw < ERROR_BIT`, and the WaitFuture success word `0` is a token under `is_handle` too — verified against `system/src/error.rs:101-109`); error path `IpcReceiveError::from(ErrorCode::from_code(raw))` reproduces today's `IpcReceiveError::from_reg` chain (`usrlib/src/syscall.rs:46-47` + `system/src/ipc.rs:110-116`). `ipc_send` builds the envelope via `Message::new(connection.packed(), data)` (C3 fn name at this commit: `new_packed`; renamed in C6).

`usrlib::ipc::ReceivedMessage` (new): wraps the wire `Message`; `pub fn connection(&self) -> Option<UserConnectionHandle>` (decodes `self.message.conn()` via the codec — the server's reply-connection token, legitimately issued by the kernel inside the envelope), `pub fn data(&self) -> &[u8; MESSAGE_PAYLOAD_BYTES]`. `ipc_receive_value` (`usrlib/src/ipc.rs:29-35`) adapts to the `Option`/`ReceivedMessage` shapes with `NoHandle` fallback.

### 3.7 Apps (BR-1 mechanical adaptations only)

- `apps/shell/src/shell.rs`: `pi/snake/tetris/conway/tests` (:106-124) — pattern `if let Some(fh) = Syscall::load(PI_ELF) { Syscall::wait_future(fh); }` (and exec twin). Behavior-equivalent on every reachable path (today a `None`-sentinel followed by wait is a kernel panic; apps never see it — Step-14 risk note).
- `apps/test_suite/src/app.rs` (:8-24): same `if let Some(fh)` pattern at all five sites.
- `apps/random_gen_server/src/main.rs` (:39-43):
  `if let Ok(received) = Syscall::wait_for_message(fh) { let value = self.next() as usize; if let Some(connection) = received.connection() { let _ = ipc_send_value(connection, value); } }` (loop semantics unchanged).
- `apps/shell/src/shell.rs::random()` — see Step 14 (R9 both sides).

---

## 4. Steps (numbered, execution order)

### Commit C1 — Drop unused collections dependency from usrlib
**Step 1.** `usrlib/Cargo.toml`: delete line 10 `collections = { path = "../collections" }` (C4-fact: zero `collections` references in `usrlib/src/**` today; no source change).
Why: R11's usrlib edge dies first; trivially wire-atomic.
Gate: G4. *(Structural: `! grep -RnE 'extern crate collections|use collections' usrlib/src` — already empty; re-run as a tripwire.)*

### Commit C2 — Relocate generational arena into kernel
**Step 2.** Create `kernel/src/collections/mod.rs` (`pub mod arena;`) and `kernel/src/collections/arena.rs`: content = `collections/src/generational_arena.rs` verbatim **except** the test-module import line `use crate::generational_arena::{Error, GenerationalArena, Handle, ERROR_BIT};` → `use crate::collections::arena::{Error, GenerationalArena, Handle, ERROR_BIT};` (the `alloc::collections::VecDeque` import stays — no clash: `alloc::` resolves to the extern crate, the module is `crate::collections`). Keep `pub struct Handle { pub index, pub generation }` with its pub fields and `pub const ERROR_BIT` **for now** (pure move, R1: "Kernel behavior unchanged by the move"). Update `kernel/src/lib.rs`: delete line 5 `extern crate collections;`, add `pub mod collections;`. Update `kernel/Cargo.toml`: delete line 9 `collections = { path = "../collections" }`. Kernel deps become `system` + `lazy_static` (R11 net-neutral).
**Step 3.** Rewrite the remaining 10 kernel consumer imports (C1 list, Context Map §4 table): `use collections::generational_arena::{GenerationalArena, Handle}` (and the `Handle`-only / `Error`-only variants) → `use crate::collections::arena::{…}` in: `ipc/ipc_manager.rs:4`, `ipc/mailbox_manager.rs:4`, `task.rs:1` (+ test `:197` arena use), `future.rs:6`, `task_manager.rs:1` (+ test `:101`), `kernel.rs:22`, `scheduler/fifo_strategy.rs:1`, `scheduler/mlfq_strategy.rs:2`, `scheduler/scheduler.rs:219` (test), `scheduler/timer.rs:39` (test). Semantics identical; `task.rs:10` stays `pub(crate) type TaskHandle = Handle;` (now arena-local Handle).
Why: kernel leaves the collections crate; `system` still consumes the crate copy — two closed worlds (BR-2). Kernel unit tests (182 + 21 relocated) prove the move.
Gate: G3 (zero failures; total re-baselines — do not assert), G6. Tripwire grep: `! grep -RnE 'extern crate collections|use collections::' kernel/src` (allowing `alloc::collections` — pattern requires `use collections::`, which no legitimate file contains).

### Commit C3 — Define wire ABI constants and raw token codec in system
**Step 4.** `system/src/error.rs`: replace line 4 `pub use collections::generational_arena::ERROR_BIT;` with the direct definition `pub const ERROR_BIT: usize = 1 << 31;` (value-identical — R8); add `pub const fn is_token(raw: usize) -> bool { raw < ERROR_BIT }` (verbatim predicate of `is_handle`). Update `system/src/ipc.rs` test import `use collections::generational_arena::ERROR_BIT;` (:184) → `use crate::error::ERROR_BIT;`. Still present (untouched this step): `Handle` imports, aliases, `into_reg`/`from_reg`, `ErrorCodeOf` (C6 removes).
**Step 5.** `system/src/ipc.rs`: add, alongside the existing pair, the raw treaty pair `pub fn new_packed(conn: u32, data: &[u8; MESSAGE_PAYLOAD_BYTES]) -> Self` and `pub const fn conn_packed(&self) -> u32` (field-forward). No removals; the old pair's behavior untouched (R8).
Why: additive ABI so kernel (C4) and usrlib (C5) migrate without ever importing the collections-typed pair after their commits; avoids re-adding a kernel→collections edge via `Message` constructors.
Gate: G2.

### Commit C4 — Introduce kernel handle domain, Future trait, and IPC error vocabulary
**Step 6.** Create `kernel/src/handle.rs` per §3.3: move `Handle` (fields → `pub(crate)`, `const fn new` + debug asserts verbatim, `pack` with debug assert verbatim, split helpers, `next_generation` verbatim), layout consts derived from `system::error::ERROR_BIT` (BR-3), and the four `pub` newtypes with the `pub(crate)` surface (`new`, `pack`, `from_packed`, `index`, `generation`, `From` pairs). `kernel/src/lib.rs`: add `pub mod handle;`. `kernel/src/collections/arena.rs`: `use crate::handle::{Handle, GENERATION_MASK};` — storage + `Error` only; `remove`'s gen bump calls `crate::handle::next_generation`.
**Step 7.** Kernel `Future` trait: move `system/src/future.rs:7-20` verbatim (incl. BR-7 doc comment) to `kernel/src/future.rs`. Move `IpcMessageFuture` (`system/src/ipc.rs:133-179`) verbatim to `kernel/src/ipc/message_future.rs` against the kernel trait and kernel error enum; `kernel/src/ipc/mod.rs`: `pub(crate) mod message_future; mod errors;`.
**Step 8.** `kernel/src/ipc/errors.rs` per §3.4: four kernel enums + four `impl From<…> for system::error::ErrorCode` projections (mapping table `system/src/error.rs:48-92`). Kernel copies drop `to_reg`/`from_reg`; keep `Display` only if a kernel call site renders one (grep says none in prod kernel — drop; usrlib mirrors carry Display in C5).
**Step 9.** Update every kernel consumer (§3.3 full list): `system::future::{Future, FutureHandle}` → `crate::future::Future` + `crate::handle::FutureHandle`; `system::ipc::{IpcConnectionHandle, IpcBindingHandle, IpcMessageFuture}` → `crate::handle::*` + `crate::ipc::message_future::IpcMessageFuture`; the four error enums → `crate::ipc::errors::*`; full-path sites `kernel.rs:63`, `mailbox_manager.rs:12-16`, `ipc_manager.rs:13` included. `task.rs:10` → `pub use crate::handle::TaskHandle;`. `mailbox_manager.rs:21` stays `pub(crate) type MailboxHandle = Handle;` (BR-9). Newtype wrap/unwrap at arena seams (`From` pairs) in `ipc_manager`, `task_manager`, `future.rs::FutureRegistry`. Scheduler test helpers (`fifo_strategy.rs:57`, `mlfq_strategy.rs:101`, `scheduler.rs:248`, `timer.rs:41`) → `TaskHandle::new` / `FutureHandle::new`. `ipc_manager.rs:166-167` consts → `TaskHandle::new(1, 1)` style; `ipc_manager.rs:327` `Handle::new(99, 1)` → `IpcBindingHandle::new(99, 1)`. `Message` envelope sites → `new_packed`/`conn_packed` (Step 5 fns); `send_message` decode per §3.5. Kernel test assertions on `delivered.conn()` per §3.5.
**Step 10.** `kernel/src/syscall.rs` kernel edge per §3.5 (delete `use system::error::into_reg;`; add `use system::error::ErrorCode;`). Exec/LoadElf sentinel literals untouched (BR-4).
Why: kernel-side half of R6 + all of R2/R3/R4/R10 for the kernel world. Wire behavior unchanged for every legitimate word (BR-5 documents the only delta, on malformed input).
Gate: G3, G4, G6, **G8, G9** (usrlib + system old surface untouched → wire identical; ipc_random with its *old* numeric assertions still green because shell still prints `.index`/`.generation` via the still-existing system aliases). Tripwire greps: `! grep -RnE 'system::future|system::error::into_reg|system::ipc::\{[^}]*(Handle|MessageFuture|Error)' kernel/src`.
Blast-radius processes: §6.3 — run G3 single-threaded (global services registry) and both QEMU gates; these transitively exercise `FutureRegistryNotifierUseCase` → downcast → `IpcMessageFuture` through `kernel_services` (indirect plumbing this step replaces).

### Commit C5 — Move user edge to opaque handles and mirrored errors
**Step 11.** `usrlib/src/handle.rs` + `usrlib/src/lib.rs` module add per §3.6.
**Step 12.** Mirror enums per BR-6 + `ReceiveValueError::NoHandle`; `usrlib/src/ipc.rs` (`ipc_send_value`, `ipc_receive_value`, `ReceivedMessage`) per §3.6.
**Step 13.** `usrlib/src/syscall.rs` signature flip per §3.6; every decode via the codec (no bit ops — gate §7.3). Imports shrink to: `system::ipc::{Message, MESSAGE_PAYLOAD_BYTES}`, `system::error::{ErrorCode, from_code via call, is_token}`, `system::syscall_numbers::SyscallNum`, crate-local `handle::*` — the C3-superseded list in Context Map §5/§7 ends here (C3-of-Context-Map items covered).
**Step 14.** Apps adaptation (BR-1) + **R9 both sides, one commit**:
  - `apps/shell` load/exec sites + `apps/test_suite` sites per §3.7.
  - `apps/shell/src/shell.rs::random()` (replaces `:130-154`): keep loop `1..260`; record the first `Ok` connection in `let mut first: Option<UserConnectionHandle>`; the per-iteration print becomes `println!("RANDOM Server: {}", ipc_connection);` (Display = `{:08x}` packed hex — R5); keep the inner send/receive prints **verbatim** (`"RANDOM Value requested"`, `"RANDOM Value: {}"`, `"RANDOM Value not received: {}"`, `"RANDOM Failed to send: {}"`, `"Connection failed"`). After the loop: if `first` is `Some(stale)` — `match ipc_send_value(stale, 0) { Err(_) => println!("RANDOM Stale rejected"), Ok(()) => println!("RANDOM Stale accepted") }` — then `if let Ok(fresh) = Syscall::ipc_connect("RANDOM")` { existing send/receive print chain; success prints `println!("RANDOM Fresh value: {}", value)`; disconnect fresh }.
  - `tests-integration/tests/ipc_random.rs`: `SERVER_PREFIX` stays `"RANDOM Server: "`. Replace decimal pair parsing (:33-45, `server_pairs`) with single-token hex parsing (`usize::from_str_radix(rest.trim(), 16)`); keep the three `count_prefixes == ITERATIONS` assertions (:97-114) verbatim; replace the numeric-growth assertion (:116-130) with machine-matchable property assertions: `content.contains("RANDOM Stale rejected")` and `count_prefixes(&content, "RANDOM Fresh value: ") == 1`; keep the consecutive-value-inequality assertion (:132-145) and the `FAILURE_STRINGS` sweep (:147-153) — extend FAILURE_STRINGS with `"RANDOM Stale accepted"`. `window_diagnostics` adapts to hex (rename `first_generation`→`first_packed` style; diagnostics only). Rationale (BR-10): stale rejection after slot recycling + fresh success is a strictly stronger property than numeric growth and depends on zero kernel-internal structure — the hex word is the opaque token itself.
Why: completes the user half of R6, R7, R5, R9, OD1, OD2. System still carries its dead-until-C6 domain surface (BR-2).
Gate: G4, G5, **G8, G9**. Tripwires: `! grep -RnE '\.(index|generation)\b' apps usrlib --include='*.rs'`; `! grep -RnE '(<<|>>)' usrlib/src`; `! grep -RnE 'ERROR_BIT|INDEX_MASK|GENERATION_MASK|is_handle' usrlib/src`.

### Commit C6 — Reduce system to the ABI treaty and dissolve the collections crate
**Step 15.** `system/src/ipc.rs`: delete `use collections::generational_arena::Handle;`, both alias types, the four error enums (+ their `to_reg`/`from_reg`/`Display` — kernel/usrlib own their copies), `use crate::error::{ErrorCode, ErrorCodeOf};`; delete old `Message::new`/`conn`; **rename** `new_packed`→`new`, `conn_packed`→`conn` (update the kernel test sites, `usrlib` sites `ipc_send`/`ReceivedMessage`, and system tests in this commit). Rewrite tests per §5 (layout/offset tests verbatim; roundtrip test uses raw `0x7FFF_FFFF`; disjointness test rewritten to raw-`usize` law: `is_token(0x7FFF_FFFF)` true, every `ErrorCode::to_reg()` has bit 31 and `!is_token` it).
**Step 16.** `system/src/error.rs`: delete `use collections::generational_arena::Handle;`, `mod private`, `trait ErrorCodeOf` + 4 impls, `into_reg`, `from_reg` (R6). Rewrite tests per §5. `system/src/lib.rs`: delete `extern crate collections;` (line 3) and `pub mod future;` (line 6); delete `system/src/future.rs`. `system/Cargo.toml`: delete the collections dep.
**Step 17.** Delete the `collections/` directory; root `Cargo.toml`: remove members line 3 `"collections",` and default-members line 21 `"collections",` (BR-12: 16→15 / 8→7).
Why: R1/R2/R11 completion; **the R2 grep gate (§7.2) reads zero for the first time here — that is expected, not a violation at earlier commits (BR-2)**.
Gate: **full chain** G2..G9 (G1 retired — package gone), grep gates §7.2, plus structural: `! test -d collections`, `! grep -nE '"collections",' Cargo.toml`, `cargo test -p collections` must fail with "package ID specification … did not match" (absence proof; expected non-zero — verifier records it as the gate, it is intentionally not green).

### Commit C7 — Add negative compile check for user handle kinds
**Step 18.** Create `checks/compile_fail.sh` (POSIX sh) implementing the injection-style probe (third-party `trybuild`-class dev-deps rejected: no crates.io deps exist in this workspace today and kernel-side deps bans make the ecosystem assumption hostile; the scripted check is the delegation-sanctioned alternative):
1. Back up `apps/shell/src/shell.rs`.
2. Append snippet A: `fn _neg_kind_confusion(c: usrlib::handle::UserConnectionHandle) -> usrlib::handle::UserFutureHandle { c }` — must fail `error[E0308]`.
3. Append snippet B: `fn _neg_wrong_api(b: usrlib::handle::UserBindingHandle) { let _ = usrlib::ipc::ipc_send_value(b, 7); }` — binding where a connection belongs must fail `error[E0308]`.
4. Run the shell-compiling build with each snippet: expected non-zero exit + stderr contains `error[E0308]`; then `git checkout -- apps/shell/src/shell.rs` and run the same build expecting zero (positive sanity restored).
**BASELINE PIN (2026-10-11): `cargo xtask apps` does NOT compile shell** (`USER_APPS` at `xtask/src/main.rs:5` lists the 5 ELF apps only). Shell is a **library dependency of both kernel binaries** (`arch/x86_64/Cargo.toml:11`, `arch/x86_32/Cargo.toml:9`), compiled for the kernel targets during `cargo xtask build` (and G7b). The script must therefore use `cargo xtask build` (or the narrower bare `rosx`-bin build) as the probe build — NOT `cargo xtask apps`. The script itself stays `unverified` until the coder authors and exercises it at C7 (self-restoring: backup → inject → probe → `git checkout --` restore → positive re-run).

### Commit C8 — Document kernel-owned handle architecture (R14/R15)
**Step 19.** `README.md`:
- `:64` — replace "dependencies limited to `collections`, `system`, `lazy_static`" with "dependencies limited to `system` and `lazy_static`".
- `:181` — remove the `├── collections/            # Custom collections` tree entry; the kernel entry gains "collections (arena) + handle (domain newtypes)".
- `:198` — same deps-list replacement as `:64`.
- New "Architecture principles" section: the five R15 principles, each with the one-line rationale (paraphrase of spec R15.1–R15.5: domain knowledge owned by the allocating subsystem — OS policy must not live in infrastructure crates; kernel is the single invariant authority — user space carries opaque tokens, never re-checks kernel invariants; the shared treaty holds the contract, not the mechanism — a type describing *how the kernel works* is in the wrong crate; translation lives at the boundaries — dependency arrows forbid the shared crate owning either direction; user-visible APIs expose no kernel internals — debug aids are kernel-side, never ABI), plus the regression-guard sentence from spec R15 verbatim-equivalent (the inversion survived because no document stated who owns handle semantics; the HAL rule — written down — was obeyed).
**Step 20.** `AGENTS.md`:
- `:34` — remove `├── collections/ # generational_arena` from the tree; annotate the `kernel/` block with `collections/ (arena)  handle/ (4 newtypes: task/future/ipc-connection/ipc-binding)`.
- `:74` — delete `cargo test -p collections      # generational_arena` (arena tests now run under `cargo test -p kernel`).
- `:82` — replace the stale `cargo xtask test [--integration] # Run unit tests; --integration is reserved` with `cargo xtask test [--skip-integration] # Run unit tests; --skip-integration excludes the QEMU tests-integration suite` (fact: `xtask/src/main.rs:146`, `ci.yml:31`).
- `:206` — replace "dependencies are limited to the workspace crates `collections` + `system` and `lazy_static`" with "dependencies are limited to the workspace crate `system` plus `lazy_static`".
- Same "Architecture principles" section as README (five principles + one-line rationales + translation-placement law + token-carrier rule); update the "Build & Run"/structure snippets that still show `collections/`.
Gate: §7.5 doc greps.

---

## 5. Test relocation (contract; final counts re-baselined by verifier — BR-13)

**System's 12** (Context Map §3; destinations binding):

| Test (baseline location) | Destination | Form |
|---|---|---|
| error.rs:129 ok-handle codec roundtrip | `kernel/src/handle.rs` tests (C4) | encode edge pairs via `TaskHandle::new(...).pack()`, decode via `from_packed` + `is_token` |
| error.rs:138 every ErrorCode through register | `system/src/error.rs` (kept) | rewritten `to_reg` + `from_code` + `!is_token(raw)` (Handle-free) |
| error.rs:148 connection mapping | `kernel/src/ipc/errors.rs` tests (C4) | `From<IpcConnectionError> for ErrorCode` + wire decode |
| error.rs:169 binding mapping | `kernel/src/ipc/errors.rs` tests | same shape |
| error.rs:179 send mapping | `kernel/src/ipc/errors.rs` tests | same shape |
| error.rs:198 receive mapping | `kernel/src/ipc/errors.rs` tests | same shape |
| error.rs:220 from_code totality | `system/src/error.rs` (kept) | verbatim (already domain-free) |
| error.rs:228 sentinel/high-bit rejection | `system` predicate twin (`!is_token(1 << 40)`, `!is_token(u64::MAX as usize)`) **plus** new `usrlib` unit test `UserFutureHandle::from_token(u64::MAX as usize) == None` (R6: each half unit-tested in its own crate) | split |
| ipc.rs:186 Message 64B/align4 | `system` (kept) | verbatim |
| ipc.rs:192 offset conn=0/data=4 | `system` (kept) | verbatim |
| ipc.rs:198 Message roundtrip | `system` (kept) | rewritten raw: `Message::new(0x7FFF_FFFF, &pattern)` / `conn()` |
| ipc.rs:207 disjointness law | `system` raw twin (`is_token(0x7FFF_FFFF)`, every code's `to_reg()` not a token) **plus** kernel twin naming `TaskHandle::pack()` in `kernel/src/handle.rs` tests | split |

**Collections' 21**: ride into the kernel at Step 2; C4 splits them — domain/layout tests (pack encoding :134, unpack :140, roundtrip :148, edge roundtrip :154, stride-997 never-sets-ERROR_BIT sweep :161-178, is_handle boundary set :180-188, 32768-iteration cycle :190-202, next_generation wrap :204-209, `#[cfg(debug_assertions)] should_panic` :211-216, equality :219) → `kernel/src/handle.rs` tests against the newtypes (`unpack`-based bodies adapt to `from_packed` + `index()/generation()`; sweep runs over one representative newtype — kernel-internal permission); storage tests (slots :229, borrow/borrow_mut/remove/full/consistency/complex-type/zero-S panic/round-robin) stay in `kernel/src/collections/arena.rs` tests. The cfg-gated panic test keeps its `#[cfg(debug_assertions)]` gate and the exact `expected = "generation outside 15-bit handle space"` string.

**New tests mandated by this plan** (beyond relocation): usrlib mirror-mapping totality test (every `ErrorCode` variant + an unknown raw maps through each mirror's `From`, total, catch-all per BR-6); usrlib decode `None` tests (sentinel + high-bit garbage); `ipc_random` property assertions (Step 14).

---

## 6. Risks and edge cases (coder must handle)

1. **Feature-unification pitfall on bare builds (G6).** The `--features lazy_static/spin_no_std` flag is load-bearing and single-package-scoped (cargo unifies features only across packages named in the invocation). Never drop it, never "simplify" it to `--features spin_no_std`, and never run the bare kernel build as part of a multi-package invocation.
2. **`Message::conn` raw-u32 ripple.** Sites: `ipc_manager.rs:121` (decode in `send_message`, BR-5), kernel test comparisons (`ipc_manager.rs:201,214,217,221,275,300`; `mailbox_manager.rs:143,171` — `Handle::new(1, 1)` construction → newtype/`new_packed` forms), usrlib `ipc_send` envelope build, `random_gen_server` reply path (Step 14), system roundtrip test. Forgetting one silently re-introduces a domain-typed conn (C6 then fails the build — loud).
3. **`IpcMessageFuture` relocation blast radius.** It is wait-queue payload registered behind `Box<dyn Future + Send + Sync>` and recovered by downcast — **indirect plumbing via the global services registry**. Transitive users (grep-verified): `kernel/src/syscall.rs:53` (WaitFuture downcast), `kernel/src/kernel_services.rs:45` (`FutureRegistryNotifierUseCase::complete_ipc_message` downcast), `ipc/ipc_manager.rs` tests (downcast sites :187, :291, :314, :330, :353), `ipc/mailbox_manager.rs` tests (:169) — plus every test reaching them via `services().future_registry` / `init_services()` (`mailbox_manager.rs` tests :150-204, `ipc_manager.rs` `manager_with_notifier`). Proof obligation: G3 **with `--test-threads=1`** (shared global state) + G8 + G9 at C4 and C6 — not production-path compilation alone.
4. **Generation-wrap under newtypes.** The wrap law now lives behind the newtypes: the 32768-iteration cycle test and the `next_generation` wrap test must run through the newtype/arena pairing (fresh handle after wrap has generation `0` and `pack() & ERROR_BIT == 0`); debug asserts must not fire in dev-profile QEMU runs — wrap policy is what keeps the domain invariant true at the assert sites (`remove` → `next_generation`); the `should_panic` test stays `#[cfg(debug_assertions)]`-gated so release-profile runs don't expect a panic.
5. **Non-canonical inbound words (BR-5).** `from_packed` failure paths per §3.5; the aliased-conn hardening delta is intentional and unexercised by any gate except the kernel unit test you add for it (assert `send_message` with a conn word carrying `ERROR_BIT` set ⇒ `Err(IpcSendError::ConnectionNotFound)`).
6. **Exec-failure sentinel path.** `schedule()` failure today panics in kernel before the sentinel is ever waited on; the app-side `if let Some` change must not swallow the success path — verify G8/G9 (`shell ls`, `random` sweep, `tests` command are exercised; test_suite app still boots and prints its markers).
7. **QEMU harness coupling.** Shell print format and test parsing must land in the same commit (C5) or G9 reds at an intermediate boundary. New deterministic lines ride inside the existing prompt-return detection; runtime growth is +2 roundtrips (prior runs ~22s vs 180s `SWEEP_TIMEOUT` — headroom confirmed). `FAILURE_STRINGS` sweep must still hold: `"Connection failed"` stays absent after the extra fresh connect.
8. **Warning multiset drift from visibility (BR-8).** `TaskHandle` visibility widening and deleted warning sites may *decrease* per-code counts (e.g. E0446-family private-interface lints); attribute via the §7.4 multiset diff — decreases are expected at deleted sites, never violations; **any new code or per-target increase is a defect**.
9. **`alloc::collections` false positives.** The kernel and apps legitimately use `alloc::collections::{BTreeMap, VecDeque}`; every collections-removal grep below is anchored to `extern crate collections|use collections::` — never bare `collections`.
10. **Doc/build scripts referencing the crate.** `cargo test -p collections` appears in docs (Step 20); any hidden CI/script reference: the C6 gate set includes CI parity (G4 = ci.yml:31 job exactly) so CI-equivalence is proven locally; `build-artifacts.yml` is x86_64 `xtask build` only (C10) — unaffected.
11. **Edition-2024 paths.** `use alloc::collections::VecDeque;` inside `kernel/src/collections/arena.rs` resolves via the extern prelude; the new crate module is `crate::collections` — never import `collections::` bare inside the kernel (would hit the new module or fail; gate tripwire in Step 3 catches misuse).

---

## 7. Acceptance criteria (final tree; all must pass)

### 7.1 Build + test gates (exact commands)
1. `cargo test -p system` — zero failures; totals as re-baselined (R12).
2. `cargo test -p kernel -- --test-threads=1` — zero failures; totals as re-baselined (includes relocated arena/handle suites; the cfg(debug_assertions) panic test must run and pass under the host dev profile).
3. `cargo xtask test --skip-integration` — zero failures; totals as re-baselined; `cargo test -p collections` must fail with a package-not-found error (crate dissolved — recorded, expected non-green by design).
4. `cargo xtask build`
5. `cargo build -p kernel --features lazy_static/spin_no_std --target arch/x86_64/rosx.json -Zbuild-std=core,alloc,compiler_builtins -Zbuild-std-features=compiler-builtins-mem -Zjson-target-spec`
6. `cargo build --release -p hello_elf -p random_gen_server -p snake -p tetris -p conway --target arch/x86_32/rosx-i686-user.json -Zbuild-std=core,alloc,compiler_builtins -Zbuild-std-features=compiler-builtins-mem -Zjson-target-spec` (BASELINE PIN: `--release`, see §0.2)
7. `cd arch/x86_32 && cargo build -p rosx-x86` — **only after 6** (load-bearing order).
8. `cargo test -p tests-integration --test boot_and_shell` — 4 passed.
9. `cargo test -p tests-integration --test ipc_random` — passed, with the R9 property assertions (stale rejected / fresh works) green.

### 7.2 R2 grep gates (portable POSIX `grep -E`, all must produce **no output**)
```
! grep -RnE 'extern crate collections|use collections|generational_arena' system/src
! grep -RnE '\bHandle\b' system/src
! grep -RnE '\bFuture\b' system/src
```
Word-aware rationale (C5): `\bFuture\b` cannot match the surviving variant names `WaitFuture` / `IsFutureCompleted` (`system/src/syscall_numbers.rs:9-10` — `t` is a word char before `F`); `extern crate collections` is matched by the first pattern (`system/src/lib.rs:3` form — a `use collections` pattern alone misses it, C2); `IpcMessageFuture` is gone from `system` entirely, so no survivor collides. Case-sensitive matching is intended (lowercase `is_token` etc. are not type names).

### 7.3 R4 / R5 / R13 negative gates (no output)
```
! grep -RnE '\.(index|generation)\b' --include='*.rs' apps usrlib          # R4/R5: no field access anywhere user-side (baseline sole site shell.rs:134 gone)
! grep -RniE '\b(index|generation)\b' --include='*.rs' usrlib/src apps     # R5: no decomposition-shaped text (literals, Debug keys) user-side
! grep -RnE '(<<|>>)' usrlib/src                                            # R13: usrlib does zero bit arithmetic
! grep -RnE 'ERROR_BIT|INDEX_MASK|GENERATION_MASK|is_handle' usrlib/src    # R13: no kernel/wire layout constant names in usrlib (codec call into system is the sole contact)
! grep -RnE 'system::future|system::error::into_reg' kernel/src            # kernel off the evicted surface
```
Plus the type-shape proof: `checks/compile_fail.sh` green (Step 18; `unverified` until the baseline pass pins it).

**ADJUDICATION (review round 1, 2026-10-11):** line 2 (case-insensitive `\b(index|generation)\b` over `usrlib/src apps`) is unsatisfiable as authored for reasons predating the series — application vocabulary (conway generations, tetris cell indices, allocator loop indices) already produced 13 matches at the pre-refactor commit `3de2324`. The enforced scope is `usrlib/src` (zero matches on the final tree) plus the line-1 field-access gate over `apps` (clean; the sole baseline site `shell.rs:134` is gone). Gate-definition defect; no code change required.

### 7.4 Warning fingerprint gate (Δ-new = 0 against the **re-established** baseline, BR-15; manual bookkeeping — no repo tooling exists, Context Map §8)
Baseline capture (verifier, branch-point commit **and** re-run on final tree):
```
cargo clean
cargo xtask build                                              2>/tmp/wf/g5.log
cargo build -p kernel --features lazy_static/spin_no_std --target arch/x86_64/rosx.json -Zbuild-std=core,alloc,compiler_builtins -Zbuild-std-features=compiler-builtins-mem -Zjson-target-spec 2>/tmp/wf/g6.log
cargo build --release -p hello_elf -p random_gen_server -p snake -p tetris -p conway --target arch/x86_32/rosx-i686-user.json -Zbuild-std=core,alloc,compiler_builtins -Zbuild-std-features=compiler-builtins-mem -Zjson-target-spec 2>/tmp/wf/g7a.log
(cd arch/x86_32 && cargo build -p rosx-x86)                    2>/tmp/wf/g7b.log
cargo test -p tests-integration --no-run                       2>/tmp/wf/gti.log
```
Counting filter (must match **both** warning forms — a plain-`^warning:`-only pattern silently undercounts code-bearing diagnostics; E0133 pitfall, Context Map §8):
```
for f in /tmp/wf/*.log; do grep -hoE '^warning:|warning\[[A-Z][0-9]+\]:' "$f" | sort | uniq -c; done
```
Rule: per (target, warning-code-or-blank) multiset, Δ-new = 0 — no new code/target pair, no count increase at a surviving site. **Decreases are expected at legitimately deleted sites**, attributed by multiset diff against the recorded baseline (e.g. warnings inside `system/src/future.rs`, `collections/`, deleted codec tests, E0446-family lints removed by BR-8). Never absolute-zero over a warning-bearing baseline; never count-equality. `unverified` as a composite (the four build commands are blessed; `cargo test -p tests-integration --no-run` and the counting pipeline are Context Map §8's recommended procedure, not a proven-blessed script) → routed to the orchestrator's baseline pass.

### 7.5 Documentation gates (no output)
```
! grep -nE 'cargo test -p collections' AGENTS.md
! grep -nF '[--integration]' AGENTS.md
! grep -nF '`collections`, `system`' README.md AGENTS.md
! grep -nE '^\s*├── collections/' README.md AGENTS.md
```
Structural: workspace has 15 members / 7 default-members; `collections/` directory absent; `plans/` untouched except this document.

---

## 8. Unverified register (routed to the orchestrator's baseline pass; never binding until that transcript proves them)

| Item | Status after orchestrator baseline pass (2026-10-11) |
|---|---|
| `checks/compile_fail.sh` (Step 18) | Probe-build question PINNED (shell compiles via `cargo xtask build`, not `cargo xtask apps` — see Step 18 pin). Script itself still unverified: it does not exist until the coder authors it at C7; coder must exercise it end-to-end (inject→E0308→restore→green) as C7 gate evidence |
| §7.4 warning-capture composite (incl. `cargo test -p tests-integration --no-run`) | **PROVEN GREEN at baseline** (all five capture commands exit 0 with G7a(release) inserted; fingerprint recorded in §0.1). Composite is now binding |
| `cargo test -p collections` "package-not-found" expected failure (acceptance 3) | **Error text PINNED**: `error: package ID specification \`collections\` did not match any packages`, exit 101 (probe `cargo test -p no-such-package-zzz-xyz`) |
| Final post-refactor per-suite test totals | Deliberately unasserted (BR-13/R12); baseline totals recorded in §0.1: collections 21, system 12, kernel 182, shell 5, xtask total 220, boot_and_shell 4, ipc_random 1 |

All other commands above are the blessed forms of Context Map §7, green on the PR #57-lineage baseline per run-20261010-205728 logs; the verifier re-runs them on the final tree per R12.
