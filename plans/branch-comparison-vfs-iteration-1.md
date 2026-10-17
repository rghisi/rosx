# RosX Branch Comparison Report
## VFS Iteration 1 — `vfs-first-iteration` vs `experiment-plan-execution-with-a-single-agent`

**Date:** 2026-10-17
**Scope:** Source files only (`.rs`, `.toml`, `.ld`). Documentation and `.md` files excluded by request.

---

## 1. Setup and Methodology

| Item | Value |
|---|---|
| Branch A | `vfs-first-iteration` @ `8789be8` (multi-agent workflow run) |
| Branch B | `experiment-plan-execution-with-a-single-agent` @ `4722c6f` (single-agent workflow run) |
| Common ancestor | `2603690` |
| Sync state | Both branches confirmed 0 ahead / 0 behind `origin` after `git fetch` |
| Files touched by A (source) | 21 |
| Files touched by B (source) | 21 — **identical file set** |
| Tip-to-tip diff | 10 files, +995 / −1194 lines |

Both branches executed the same VFS Iteration 1 plan from the same base commit and produced near-mirror commit chains (`wire codec → server → boot scheduling → client+shell command → integration test`).

**Commit-chain structure:**

| `vfs-first-iteration` (10 commits) | `experiment-…-single-agent` (6 commits) |
|---|---|
| `bb8c09f` Record plan (first) | `18e8f40` Wire codec + client |
| `343ff0a` Plan validation/binding amendments | `709cb0b` VFS server |
| `ce13f66` Wire codec + client | `690c002` Boot scheduling |
| `4db84bf` VFS server | `1dedfe7` Client + shell command |
| `7b4b6ae` Boot scheduling | `0297044` Integration test |
| `03564cf` Client + shell command | `97724a1` AGENTS.md tree |
| `a595b14` Integration test | `4722c6f` Record plan (**last**) |
| `59ca279` AGENTS.md tree | |
| `c186f3f` Encode run lessons into specs | |
| `8789be8` Archive run | |

Record-keeping differs (plan recorded first + run archive vs plan recorded last, no archive), but code commits map 1:1.

---

## 2. The Shared Wire Protocol (both branches)

Both implementations agree byte-for-byte on the happy path:

- **Frame:** fixed `[u8; 60]`, little-endian
- **Ops:** `OPEN=1, READ=2, WRITE=3, CLOSE=4, DELETE=5, LIST=6`
- **Status:** `OK=0, ERR=1, EOF=2`; error codes `NotFound=1 … ServerResource=7`
- **Layouts:** Open = `[op, mode, len, name…]`; Delete = `[op, len, name…]`; Read/Write = `handle@1..5, offset@5..13, count|token@13..17`; Close = `handle@1..5`; List = op byte only
- **Contract constants:** `RPC_MAX_DATA_BYTES = 64 KiB`, `MAX_NAME_BYTES = 57` (names reject NUL/newline, service `"VFS"`)
- **Data plane:** message frames carry tokens; bulk payloads ride the IPC data objects (`ipc_data_write`/`ipc_data_read`, 4 KiB drain chunks), server retains reply payload objects (`held`) until the owner's next message

---

## 3. File-by-File Comparison Matrix

### 3.1 Byte-identical files (11 of 21)

| File | Content |
|---|---|
| `Cargo.toml` (workspace) | Adds `vfs-protocol`, `apps/vfs_server`, `apps/vfs_client` members |
| `apps/shell/build.rs` | Embeds `vfs_client` ELF |
| `apps/shell/src/shell.rs` | `vfs` shell command |
| `apps/vfs_client/Cargo.toml` | PIE user app manifest |
| `apps/vfs_client/build.rs` | Linker-script link args for `target_os = "none"` |
| `apps/vfs_client/linker.ld` | PIE linker script |
| `apps/vfs_server/linker.ld` | PIE linker script |
| `apps/vfs_server/src/lib.rs` | `#![no_std]` + `pub mod fs` |
| `arch/x86_64/src/main.rs` | Schedules `vfs_server` ELF at boot |
| `vfs-protocol/Cargo.toml` | `no_std` + `alloc` crate manifest |
| `xtask/src/main.rs` | Builds the two new apps |

Despite completely different workflows, both runs converged on the identical integration surface.

### 3.2 Cosmetic-only differences (4 files)

| File | `vfs-first-iteration` | single-agent | Verdict |
|---|---|---|---|
| `apps/vfs_server/Cargo.toml` | implicit lib path | `path = "src/lib.rs"` explicit | Equivalent — Cargo resolves identically |
| `apps/vfs_server/build.rs` | `env().as_deref() == Ok("none")` | bind with `unwrap_or_default()`, then `== "none"` | Equivalent |
| `arch/x86_64/build.rs` | `Path::new(ping_server_path)` | `Path::new(&ping_server_path)` | Equivalent — clippy style only |
| `vfs-protocol/src/lib.rs` | `pub mod client; pub mod wire;` | `pub mod wire; pub mod client;` | Equivalent — declaration order only |

### 3.3 Substantive files (6)

#### `vfs-protocol/src/wire.rs` (745 lines vs 685)

| Aspect | `vfs-first-iteration` | single-agent | Functional? |
|---|---|---|---|
| Frame length | literal `[u8; 60]` | `pub const FRAME_BYTES` | No |
| Mode bits `MODE_WRITE/MODE_CREATE` | **not here** (duplicated in `client.rs` **and** `fs.rs`) | defined here; single source of truth | No (wire values equal), but ownership differs |
| Reply encoders (`encode_ack/eof/err/opened/read_data/written/listed`, `to_byte`) | **absent** — server hand-builds reply frames locally in `main.rs` | **present** — crate is a complete two-way codec | No on success path; architectural |
| `VfsError::Protocol` discriminant | implicit `= 8` | explicit `= 255` | **Latent** — neither side transmits `Protocol`, so never observed; a third-party encoder would see different bytes |
| `Request::{Open,Delete}` name type | `&[u8]`; UTF-8 validated later, server-side in `fs::validate_name` → replies `InvalidPath` | `&str`; `decode_request` rejects invalid UTF-8 → `serve()` returns early, **no reply sent** | **YES — divergence D3** (see §4) |
| EOF tail strictness | accepts EOF with non-zero tail (test pins this) | rejects non-zero EOF tail | Edge-only: only a non-canonical sender can trigger it |
| Test style | ~40 tests; hand-assembled byte fixtures pin `decode_reply` directly; exhaustive 0..=255 `from_byte` totality sweep | ~30 tests; round-trip through own `encode_*` helpers | Coverage broadly equivalent; A stronger on decoder-with-adversarial-bytes, B stronger on encoder pinning |

#### `vfs-protocol/src/client.rs` (208 vs 190)

| Aspect | `vfs-first-iteration` | single-agent | Functional? |
|---|---|---|---|
| API surface | `connect/open/delete/list/read/write/seek/close/offset/disconnect` | same | Identical on compliant server |
| `MODE_*` origin | defined locally in `client.rs` | `pub use crate::wire::{MODE_CREATE, MODE_WRITE}` | No |
| `ClientError` derives | `Debug, Copy, Clone, PartialEq, Eq` | `Debug` only | No |
| `read()` vs mismatched chunk | tolerant: `taken = min(chunk.len(), count, remaining)` | strict: `chunk.len() != count` → `Protocol` | Edge-only (buggy server) |
| `list()` vs `total` | ignores `total`, rejects empty blob | verifies `payload.len() == total` | Edge-only (buggy server) |
| `write()` ack handling | advances by `min(ack, chunk)`, then `Protocol` if `ack != chunk.len()`; `release()` per branch | single `release()` after rpc; exact `bytes == chunk.len()` required | No — same net effect on compliant server |
| Retry loop | manual counter `attempt >= 50` | `for _ in 0..50` | No |

#### `apps/vfs_server/src/fs.rs` (508 vs 408) — largest divergence

| Aspect | `vfs-first-iteration` | single-agent | Functional? |
|---|---|---|---|
| Data model | `files: BTreeMap<String,Vec<u8>>`, `opens: BTreeMap<u32,OpenEntry>`, handles from 1, monotonic | identical | Equivalent |
| Semantics: create/exist/mode rules, zero-filled holes, ownership checks, delete-while-open → `NotFound`, close-after-delete → `Ok`, shared-content handles | identical rule-by-rule; each pinned by a test in **both** branches | — | Equivalent |
| Empty write on read-only handle | `Ok(0)` (empty check precedes mode check) | same order, same result | Equivalent |
| Name validation entry | takes `&[u8]`, `validate_name()` = `from_utf8` + `wire::valid_name` → `InvalidPath` | takes `&str`, direct `wire::valid_name` | Consequence of wire.rs choice; net `InvalidPath` reachable in both |
| `list()` | **infallible** `-> Vec<u8>`; `ListTooLarge` variant **never emitted anywhere** | `-> Result<_, VfsError>`; `blob.len() > 64 KiB` → `ListTooLarge` | **YES — divergence D1** |
| `write()` overflow guard | `offset.saturating_add(len)` | `start + payload.len()` (unchecked add) | Latent robustness gap in single-agent (panics/wraps near `usize::MAX`) |
| Borrow handling | reordered lookups, no clone | `String::clone(&entry.name)` per write | Perf-only |
| Unit tests | ~33: adds unknown-mode-bits, 57-byte exact boundary, `u64::MAX` offset, delete-then-recreate | ~26: adds `ListTooLarge` 1200-file stress; realistic owner tokens (`0x0010_0001`) | Complementary; core rules covered by both |

#### `apps/vfs_server/src/main.rs` (251 vs 220)

| Aspect | `vfs-first-iteration` | single-agent | Functional? |
|---|---|---|---|
| Skeleton | `SyscallAllocator` global alloc, `conn_token`/`data_token` hex-parse, bind → accept → wait loop, per-owner `held` release before serving | identical mechanism | Equivalent |
| Control flow | each match arm **sends inline** (`self.send`) | `dispatch()` **returns** frame; one `ipc_send` choke point in `serve()` | No — same bytes emitted |
| Reply framing | local helpers `err_frame/eof_frame/opened_frame/written_frame/data_frame` | `wire::encode_*` from the protocol crate | No — byte-identical output; architectural ownership |
| Write drain failure | replies `ServerResource` | replies `BadHandle` | **YES — divergence D2** |
| Incoming drain cap | `DRAIN_MAX_CHUNKS = 16` (64 KiB) then **silently truncates** `Some(payload)` | loops until `copied == 0` (unbounded) | **YES — divergence D4** |
| `list` empty path | EOF; `store.list()` cannot error | EOF; `fs.list()` error path exists (`ListTooLarge` reachable) | Tied to D1 |
| `release_held_for` | `drain(..)` + retained-Vec rebuild | `retain_mut` | Equivalent |
| Read count bound | `count.min(RPC_MAX_DATA_BYTES)` before store | `min(count, RPC_MAX_DATA_BYTES)` | Equivalent |

#### `apps/vfs_client/src/main.rs` (237 vs 246)

Same **19-step acceptance scenario**, step-for-step, with identical check labels, identical deterministic pattern `f(i) = i*29+11 (wrapping)`, `NOTES_BYTES = 9216`, `BLOB_BYTES = 131072`, identical stdout (`vfs ok: …` / `vfs done`). Only idioms differ:

- A: `Result<(), &'static str>` with `check()` / `is_vfs_error()` helpers
- B: `failed(name) -> bool` + early returns, `let Ok(mut notes) = …` shadowing, explicit `drop(data)` before the 128 KiB read-back

**QEMU-observable behavior: equivalent.**

#### `tests-integration/tests/ipc_vfs.rs` (135 vs 134)

Both assert the same five properties on the QEMU session window: (1) exactly the 19 `vfs ok:` lines in order, (2) exactly one `vfs done` line, (3) `done` after last `ok`, (4) zero `vfs error:` lines, (5) forbidden strings `fail`/`Panic` absent; same timeouts (60 s ready, 180 s sweep). Differences are presentation only (A compares the whole slice at once and enriches diagnostics with error/done counts; B loops per-index). **Functionally equivalent.**

---

## 4. The Four Real Functional Divergences

| # | Divergence | single-agent behavior | vfs-first-iteration behavior | Reachable via |
|---|---|---|---|---|
| **D1** | `ListTooLarge` contract | enforced in `fs::list()` (> 64 KiB → error) | variant exists, **never emitted** (infallible `list()`); a huge listing would attempt a >64 KiB data object and surface as `ServerResource` (or kernel limit) instead | 1200+ file directory |
| **D2** | Write drain failure code | `BadHandle` | `ServerResource` | Client sending a bogus data token |
| **D3** | Non-UTF-8 name | rejected in `decode_request` → request **dropped silently, client hangs** | reaches fs → replies `InvalidPath` | Hand-built frame with invalid UTF-8 bytes |
| **D4** | Incoming payload > 64 KiB | drains unbounded (full payload written) | silently truncated at 16×4 KiB | Larger data object written by a malicious client |

Note the irony: the single-agent run is stricter on the *codec* boundary (D3 is its blind spot — strictness without a reply), while the multi-agent run is stricter on the *store* boundary (D1 is its blind spot — declared error never produced).

---

## 5. Why the Differences Correlate with Workflow

**single-agent (`experiment-…`):**
- Produced the protocol crate as a **complete two-way boundary treaty**: reply encoders, mode bits, and `FRAME_BYTES` all live in `vfs-protocol` — exactly the AGENTS.md principle *"the shared treaty holds the contract, not the mechanism"* and *"translation lives at the boundaries."*
- Centralized control flow (`dispatch` returns; one send site) and validation at the codec edge — consistent with one mind holding one design thread.
- Costs: thinner decoder-adversarial unit tests, a non-saturating overflow add in `write()`, drop-without-reply on invalid UTF-8, unbounded drain.

**multi-agent (`vfs-first-iteration`):**
- Produced denser adversarial unit tests (hand-built byte fixtures, 0..=255 totality sweeps) — consistent with verification-heavy stages leaving their fingerprint.
- But mechanism leaked out of the treaty: reply framing lives inside the server binary, and `MODE_WRITE/MODE_CREATE` are **duplicated in three files** (`client.rs`, `fs.rs`, wire constants absent) — the kind of contract scattering the AGENTS.md regression guard explicitly warns about.
- Left two error-contract dead ends (`ListTooLarge` unused; EOF-tail laxity pinned as spec).

**Convergence:** the identical 11 files and the byte-identical happy-path protocol show the plan itself, not the workflow, constrained the bulk of the outcome. Workflow differences surface precisely where the plan was silent: error-path semantics, encoder ownership, and constant duplication policy.

---

## 6. Verification Notes

- Both branches in sync with origin at analysis time; `origin/main` had advanced to `288f318` (irrelevant to this comparison).
- No `.ld` differences exist: both linker scripts are byte-identical.
- No `.toml` semantic differences exist (only the explicit `path` key).
- Net effect: **a compliant client cannot distinguish the two servers on the wire for any conforming request sequence**; all divergences are hostile-input or resource-limit paths.
