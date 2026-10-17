# VFS Iteration 1 — Implementation Plan (`vfs-protocol` + `vfs_server` + `vfs_client` + shell `vfs` + `ipc_vfs`)

> **Date:** 2026-10-17. **Baseline:** `master` @ `2603690` ("Add IPC Data Transfers").
> **Scope:** one new library crate, two new apps, boot + xtask + shell wiring, one integration test, one AGENTS.md line edit. `kernel/`, `system/`, `usrlib/`, `arch/x86_32/` receive **ZERO changes** (user-locked).
> **User-locked decisions:** (1) flat namespace, no directories; (2) **client-side offset** — every READ/WRITE carries an explicit offset, the server keeps no read cursor, the typed client library maintains the per-file offset; (3) open mode bits `WRITE=1`, `CREATE=2`, reads always permitted on an open handle; (4) `delete()` and `list()` are IN scope; (5) the wire contract lives in a NEW crate `vfs-protocol` — `system` stays kernel-mediated contract only; (6) client-death leak of open entries is tolerated and documented, not mitigated; (7) only `vfs_server` is boot-scheduled — `vfs_client` is a standalone ELF invoked by the user through the shell `vfs` command (snake/ping precedent).
> **Repo law:** no comments in any new Rust file (AGENTS.md); commit subjects carry no prefix; one commit per step.

---

## 0. Verified-baseline facts (re-verified this session against HEAD 2603690)

- **Data family kernel gates.** `kernel/src/ipc/data_manager.rs`: `const MAX_DATA_BYTES: usize = 64 * 1024;` and `const DATA_BUDGET_BYTES: usize = 256 * 1024;` (`:8-9`); size check is strict `>` (`:40`) so exactly 65536 is accepted; per-writer budget check `:43-46`; arena-full maps to `BudgetExceeded` (`:53`); **`write` accepts an empty payload** (no lower bound; `Vec::from(&[][..])`); `locate` rejects the writer itself with `NotReadable` (`:65-66`); `copy_out` is cursor-advancing, `Ok(0)` = end (`:71-80`); **`release` is writer-only** — non-writer caller gets `NotFound` (`:88-90`); `gc_connection` reclaims + refunds all objects of an explicitly disconnected connection (`:98-114`). `kernel/src/ipc/ipc_manager.rs`: `write_data` `:169-179`, `read_data` membership gate `caller != conn.client_task && writer != conn.client_task → NotReadable` `:192`, `release_data` `:198-204`.
- **usrlib surface (public, merged).** `Syscall::ipc_data_write(connection, payload) -> Result<UserDataHandle, DataError>` (`usrlib/src/syscall.rs:161-175`); `Syscall::ipc_data_read(token: usize, dst) -> Result<usize, DataError>` (`:177-189`) — takes a **bare `usize` token**, `Ok(0)` = end-of-data; `ReceivedMessage::{connection, data}` (`usrlib/src/ipc.rs:19-25`); `ipc_send(connection, &[u8; 60])` (`:121`); `ipc_receive_message` / `ipc_accept_message` non-blocking → future (`:139, :151`); `wait_for_message` blocking (`usrlib/src/syscall.rs:42`).
- **Handle types.** `UserConnectionHandle` is `Copy + Clone + PartialEq + Eq` with `packed()` **`pub(crate)`** and **no `Ord`** (`usrlib/src/handle.rs:38-55`); `Display` renders `"{:08x}"` of the packed u32 (`:63-67`). `UserDataHandle`: `release()` is idempotent via `packed.take()` (`:122-133`), `Drop` auto-releases (`:136-140`), `Display` = `"{:08x}"` or `"released"` (`:151-157`). The Display-hex→`u32::from_str_radix` round-trip is **proven green in production** by `apps/ping_server/src/main.rs:41-43` (BR2 of `plans/ipc-ping-demo-plan.md`).
- **usrlib conventions NOT reused by VFS (deliberate):** `send_data`/`receive_data`/`ipc_send_value` (`usrlib/src/ipc.rs:28-125`) all pin the data token to payload bytes `[0..4]` (NE); the VFS wire puts a status/op byte at `[0]`. VFS uses only `ipc_data_write` / `ipc_data_read` / `ipc_send` / `ipc_receive_message` / `wait_for_message` and assembles every frame itself.
- **Host-testable no_std pattern.** `usrlib/src/lib.rs:1` verbatim: `#![cfg_attr(not(test), no_std)]`; host unit tests live in `usrlib/src/{ipc,error,handle}.rs`. `cargo test -p kernel -- --test-threads=1` is the proven single-package host-test form (`plans/ipc-ping-demo-plan.md` §5 gate 1).
- **Workspace-app bin law on host builds.** Every existing app package declares `[[bin]] name = "…" bench = false test = false` (`apps/bulk_echo/Cargo.toml`). `cargo test --workspace` (what `cargo xtask test` runs, `xtask/src/main.rs:74-84`: `cargo test --workspace [--exclude tests-integration] -- --test-threads=1`) is green today, which **empirically proves `test = false` bins are never built or linked for the host**. This is what makes a lib+bin package safe to add.
- **App skeleton.** `apps/bulk_echo/`: `Cargo.toml` (deps `system`, `usrlib`), `build.rs` emits `cargo::rustc-link-arg=-T{dir}/linker.ld` + `--pie` **unconditionally**, `linker.ld` (`ENTRY(_start)`, PIE sections); `src/main.rs` shape: `#![no_std] #![no_main]`, `extern crate alloc;`, `SyscallAllocator` (`:12-25`), accept-loop shape `bind → ipc_accept_message → wait_for_message → handle` (`:36-46`), `#[unsafe(no_mangle)] pub extern "C" fn _start` (`:58-63`), own `#[panic_handler]` printing then looping (`:65-71`). Custom target `arch/x86_64/rosx-user.json` has `"os": "none"`.
- **Boot spawn roster (x86_64).** `arch/x86_64/src/main.rs:74-82`: `random_gen_server`, `bulk_echo`, `bulk_client`, `ping_server` ELFs scheduled via `include_bytes!("../../../target/rosx-user/release/<app>")` + `kernel.schedule(new_elf_task(…))`, then `FunctionTask::new("Shell", shell::shell::main)`. `arch/x86_64/build.rs:4-15` carries per-app `rerun-if-changed` triplets (`src`, `Cargo.toml`, `linker.ld`) and `:17-29` per-ELF existence panics.
- **Shell.** `apps/shell/src/shell.rs:31`: `#[cfg(target_arch = "x86_64")] static PING_ELF = include_bytes!("../../../target/rosx-user/release/ping_client");`; `COMMANDS` is already in the **block-form lazy_static** with a cfg'd `commands.insert(String::from("ping"), ping as fn());` (`:33-49`); spawn form (`:140-145`): `if let Some(fh) = Syscall::load(PING_ELF) { Syscall::wait_future(fh); }`. `apps/shell/build.rs`: rerun triplet for `ping_client` + existence check inside `if arch == "x86_64" { … }` (whole file verified; mirror it).
- **xtask.** `xtask/src/main.rs:5` verbatim: `const USER_APPS: [&str; 9] = ["hello_elf", "random_gen_server", "snake", "tetris", "conway", "bulk_echo", "bulk_client", "ping_server", "ping_client"];` — `build_apps()` builds each `--release` for `arch/x86_64/rosx-user.json` with the 3 `-Z` flags; `run_tests()` calls `build_apps()` first.
- **Workspace.** Root `Cargo.toml:2-22` members (includes both arches and all apps), `:23-31` default-members = `system, kernel, usrlib, apps/shell, apps/dummy, apps/test_suite, xtask`.
- **Clean task exit.** A returning `_start` completes the `LoadElf` future and wakes the shell's `wait_future` (`kernel/src/task.rs:229-241` → `terminate_and_yield`; production precedent `hello_elf` behind `pi`). An idle-loop tail would hang the shell (`plans/ipc-ping-demo-plan.md` BR3).
- **Integration harness** (`tests-integration/src/qemu.rs`): `spawn() :29-31`, `expect_output(&str, Duration) :114`, `mark() :118`, `wait_until_since(mark, pred, timeout) :126-133`, `content_since(mark) :135-137` (ANSI-stripped), `send_line(&str) :175-178`. Template: `ipc_ping.rs` (send_line-driven, prefix-count assertions, forbidden-string sweep).
- **Warning fingerprint baseline** (`.opencode/artifacts/ipc-ping-verify-final.md`): kernel lib **60**; kernel lib-test **83 (49 dup)**; shell lib **3** (now `shell.rs:148/168/182` after the ping +16 shift); rosx bin **26** + rosx manifest **1**; rosx-x86 bin **1** (`arch/x86_32/src/main.rs:66:22`); tests-integration lib **1**.
- **Name collision sweep:** `grep -ri "vfs"` over `apps/ kernel/ system/ usrlib/ xtask/ tests-integration/ arch/` = **0 matches**. `ipc_vfs` and `vfs` are free names.

---

## 1. Binding resolutions (the coder follows these verbatim)

**BR1 — untouched-file law.** `kernel/`, `system/`, `usrlib/`, `arch/x86_32/`, `arch/x86_64-runner/` and all existing app directories must not appear in any diff of this work except where a step below names the file explicitly (`xtask/src/main.rs`, `arch/x86_64/src/main.rs`, `arch/x86_64/build.rs`, `apps/shell/src/shell.rs`, `apps/shell/build.rs`, root `Cargo.toml`, `AGENTS.md`, `Cargo.lock`).

**BR2 — numeric token extraction with zero usrlib changes.** Both apps and the client library need the packed u32 of a `UserDataHandle` (and the server needs the packed u32 of a `UserConnectionHandle`) to embed in a wire frame. Use the ping-proven round-trip: `u32::from_str_radix(&format!("{}", handle), 16)` (`apps/ping_server/src/main.rs:41-43` precedent; `Display` is exactly 8 lowercase hex digits while unreleased, `usrlib/src/handle.rs:151-157`). One private helper per consumer; requires `use alloc::format;`.

**BR3 — wire contract, exact byte offsets (new crate `vfs-protocol`, module `wire`).** Frames are the 60-byte `Message` payload (`system/src/ipc.rs:2` `MESSAGE_PAYLOAD_BYTES = 60`). All multi-byte integers **little-endian** (`to_le_bytes`/`from_le_bytes`) — pinned by golden-vector tests; the crate never uses NE helpers. Byte 60 does not exist; every frame fills `[0..60]`, remainder zero.
Constants: `RPC_MAX_DATA_BYTES: usize = 64 * 1024` (= `MAX_DATA_BYTES`, the kernel per-object cap); `MAX_NAME_BYTES: usize = 57`; service name `"VFS"`.

Requests (client→server):

| op byte | frame |
|---|---|
| `OPEN = 1` | `[0]=1, [1]=mode bits, [2]=name_len, [3..3+name_len]=name bytes` (name_len 1..=57) |
| `READ = 2` | `[0]=2, [1..5]=handle u32, [5..13]=offset u64, [13..17]=count u32 (1..=65536)` |
| `WRITE = 3` | `[0]=3, [1..5]=handle u32, [5..13]=offset u64, [13..17]=data token u32` (payload = the whole data object) |
| `CLOSE = 4` | `[0]=4, [1..5]=handle u32` |
| `DELETE = 5` | `[0]=5, [1]=name_len, [2..2+name_len]=name bytes` |
| `LIST = 6` | `[0]=6` |

Replies (server→client). Status bytes: `OK = 0`, `ERR = 1`, `EOF = 2`.

| reply | frame |
|---|---|
| Ack (CLOSE/DELETE ok) | `[0]=0`, all other bytes zero |
| Err | `[0]=1, [1]=error code` |
| Eof (READ at/past end; LIST with zero files) | `[0]=2`, all other bytes zero |
| Opened | `[0]=0, [1..5]=handle u32 (≥ 1, never 0)` |
| ReadData | `[0]=0, [1..5]=data token u32, [5..9]=count u32 (≥ 1)` |
| Written | `[0]=0, [1..5]=bytes_written u32` |
| Listed | `[0]=0, [1..5]=data token u32, [5..9]=total bytes u32 (≥ 1)` |

Codec API (contract): `encode_open(mode, name) -> Option<[u8; 60]>` / `encode_delete(name) -> Option<[u8; 60]>` (`None` when `!valid_name`), `encode_read/encode_write/encode_close/encode_list -> [u8; 60]`, `decode_request(&[u8; 60]) -> Option<Request>` (strict: unknown op, name_len out of `1..=57`, or declared name extending past byte 60 → `None`), `decode_reply(&[u8; 60], expect: Expect) -> Result<Reply, ()>` where `Expect ∈ {Ack, Opened, ReadData, Written, Listed}` discriminates the otherwise-ambiguous `[1..5]` occupancies (the protocol is one-request-in-flight so the client always knows the expected kind); `Open`/`ReadData`-shape replies with a zero/absent field the Expect demands → `Err(())`. `valid_name(s: &str) -> bool` = byte length in `1..=57` and contains no `0x0A` and no `0x00` byte — **the single source of truth, called by both the client (before sending) and the server (on every wire name)**.

**BR4 — error codes on the wire (user-space space, distinct from kernel `ErrorCode`).** `VfsError` with `#[repr(u8)]` codes and total `from_byte` (unknown → `ServerResource` never; unknown byte maps to a dedicated `Protocol` variant kept internal to the client mapping): `NotFound = 1`, `AlreadyExists = 2`, `NotPermitted = 3`, `BadHandle = 4`, `InvalidPath = 5`, `ListTooLarge = 6` (joined names would exceed 65536), `ServerResource = 7` (kernel `DataError` refused the READ/LIST reply object — budget/arena).

**BR5 — release choreography (writer-only release is kernel law, `data_manager.rs:88-90`).**
- Client (writer of WRITE payloads): keeps the `UserDataHandle` from `ipc_data_write`, releases it explicitly (`let _ = h.release();`) after the Written ack arrives, before issuing the next RPC. Peak client-side outstanding: one 64 KiB object.
- Server (writer of READ/LIST reply payloads): holds returned `UserDataHandle`s in `Vec<(u32 /*owner conn*/, UserDataHandle)>`; at the **start of handling each request** it releases every held handle whose owner equals the requesting conn (implicit drain-ack, ping-style). With a synchronous client this bounds server outstanding to one reply object per live conn. Last replies of a cleanly-disconnected client are reclaimed by kernel disconnect-GC (`data_manager.rs:98-114`); a later release attempt on a GC'd token answers `NotFound` and is discarded with `let _ =`.
- Readers never own: nothing releases on the read side.
- Client-death leak (decision 6): open-file entries and held reply tokens for a dead client are never reclaimed (the kernel emits no connection-close event, and disconnect-GC fires only on the explicit `IpcDisconnect` syscall). **Documented, accepted, not mitigated.**

**BR6 — file handles are a VFS-service concept.** The server issues monotonically increasing u32 handles starting at **1** (0 is never assigned, so "zero" stays distinguishable on the wire); `opens: BTreeMap<u32, OpenEntry>` with `OpenEntry { owner: u32, mode: u8, name: String }` (`owner` = packed conn u32 per BR2, which keeps the `fs` module free of usrlib types and `Ord`-able — `UserConnectionHandle` has no `Ord`, `usrlib/src/handle.rs:38-55`). Every handle-taking op: absent handle **or** `entry.owner != requesting owner` → `BadHandle` (cross-client handle use is indistinguishable from expiry — no leak of existence info).

**BR7 — open semantics (CREATE is exclusive; plain open never truncates).**
1. name exists, `mode & CREATE == 0` → open, entry mode = requested mode.
2. name exists, `mode & CREATE != 0` → `AlreadyExists`.
3. name absent, `mode & CREATE != 0` → create empty file, open.
4. name absent, `mode & CREATE == 0` → `NotFound`.
Unknown mode bits are ignored. Reads are always permitted on a valid handle; `write` requires `mode & WRITE != 0` else `NotPermitted`.

**BR8 — fs mutation rules (`RamFs`).** `read(owner, handle, offset, count)` → clamp `start = min(offset, len)`, `n = min(count, len - start)`, return `Vec` slice; empty result ≡ EOF (RPC layer maps empty → `EOF` status; the RPC layer also rejects `count = 0` requests with `EOF` before touching fs, and clamps `count = min(count, RPC_MAX_DATA_BYTES)` before `fs.read` so a reply object can never be built past the kernel per-object cap). `write(owner, handle, offset, payload)`: empty payload → `Ok(0)` **no-op, never resizes**; else require WRITE, resize file to `max(len, offset)` zero-filling, splice payload in at offset, return `payload.len()`. `delete(name)`: remove from `files`; absent → `NotFound`; open handles to a deleted file survive the close bookkeeping but their next read/write hits `NotFound` (name lookup fails). `list()` → names in `BTreeMap` order (already sorted), joined with `'\n'`, single `Vec<u8>`; empty store → empty (RPC layer → `EOF`).

**BR9 — package shapes.**
- `vfs-protocol`: lib-only crate, `publish = false`, `edition.workspace = true`, deps `system` + `usrlib`; root attribute verbatim `#![cfg_attr(not(test), no_std)]` (usrlib pattern) + `extern crate alloc;`. Module dependency law: `wire` imports **core only**; `client` may import `wire` + `usrlib` + `alloc`. Registered in workspace `members` **and** `default-members`.
- `apps/vfs_server`: lib + bin package. `Cargo.toml` = `bulk_echo` clone **plus** a `[lib]` section (default `src/lib.rs`, name `vfs_server`) and the same `[[bin]] name = "vfs_server" bench = false test = false`; deps `system`, `usrlib`, `vfs-protocol`. `build.rs` = `bulk_echo`'s **except** the link-arg emission is gated: emit `cargo::rustc-link-arg=-T…` + `--pie` **only when `std::env::var("CARGO_CFG_TARGET_OS") == "none"`** — the host `--lib` test harness links against `vfs_server`'s package and must NOT inherit the bare-metal script. `linker.ld` verbatim clone. `src/lib.rs` = `#![cfg_attr(not(test), no_std)]` + `extern crate alloc;` + `pub mod fs;` — the `fs` module imports **alloc + vfs_protocol::wire only** (no usrlib), so its unit tests run on host with zero syscall reach. Registered in `members` (NOT `default-members` — same rule as every app).
- `apps/vfs_client`: verbatim `bulk_echo` skeleton (bin-only, `test = false`, unconditional `build.rs`, own `SyscallAllocator` + `#[panic_handler]`), deps `system`, `usrlib`, `vfs-protocol`. Registered in `members` only.
- Host-build safety: `cargo test --workspace` builds `vfs-protocol` (std under cfg(test), no_std as dependency — both proven classes), `vfs_server` lib (gated build.rs), never any app bin (§0 empirical law).

**BR10 — client API (module `client`, blocking style matching usrlib).**
```rust
pub enum ClientError { Vfs(VfsError), Transport, NoConnection, Data(DataError), Protocol }
pub struct Vfs { connection: UserConnectionHandle }
impl Vfs {
    pub fn connect() -> Result<Vfs, ClientError>                 // retry ipc_connect("VFS") up to 50x with Syscall::sleep(100) (bounded ping_client loop)
    pub fn open(&self, name: &str, mode: u8) -> Result<VfsFile, ClientError>
    pub fn delete(&self, name: &str) -> Result<(), ClientError>
    pub fn list(&self) -> Result<Vec<String>, ClientError>       // empty Vec on EOF status
    pub fn disconnect(self)                                      // Syscall::ipc_disconnect
}
pub struct VfsFile { connection: UserConnectionHandle, handle: u32, offset: u64 }
impl VfsFile {
    pub fn read(&mut self, buf: &mut [u8]) -> Result<usize, ClientError>   // chunk loop ≤ 64 KiB per RPC, stops on EOF, advances self.offset, returns total
    pub fn write(&mut self, data: &[u8]) -> Result<(), ClientError>        // chunk loop ≤ 64 KiB; per chunk: ipc_data_write → WRITE rpc → release; advances offset per acknowledged bytes; mid-loop failure returns Err with the prefix already written (no atomicity — §4 risk 3)
    pub fn seek(&mut self, offset: u64)                                    // pure client-side assignment to self.offset — state, not RPC; zero protocol surface (decision 2)
    pub fn close(self) -> Result<(), ClientError>
    pub fn offset(&self) -> u64
}
pub const MODE_WRITE: u8 = 1;
pub const MODE_CREATE: u8 = 2;
```
Internal rpc helper: `ipc_send(conn, &frame)` → `ipc_receive_message(conn)` → `wait_for_message` → payload bytes; any IPC-layer failure maps to `Transport`; `received.connection().is_none()` maps to `NoConnection`; `decode_reply` `Err(())` maps to `Protocol`; `Syscall::ipc_data_read` errors map to `Data`. `Ok(0)` from `ipc_data_read` is normal EOF, never an error (`syscall.rs:132-141` result convention).

**BR11 — server dispatch shape (`main.rs`, accept loop mirrors `bulk_echo` `:36-46`).** Order per request: conn = `received.connection()` (`None` → ignore); release held replies for that conn (BR5); `decode_request` (`None` → **ignore silently**, no reply — wire violation has no reply kind); dispatch fs op; READ/LIST: fs result empty → `EOF` frame, else `ipc_data_write` the bytes (kernel `Err` → `ERR(ServerResource)`), embed token (BR2) + count, stash handle; WRITE: drain the client's data object with `Syscall::ipc_data_read(token, &mut [u8; 4096])` until `Ok(0)` (≤ 16 chunks by the 64 KiB cap), then fs.write; reply via `Syscall::ipc_send`. Server console output is exactly the boot line (§2.6) — nothing per request.

**BR12 — x86_64-only client surface.** Everything on the client side is `#[cfg(target_arch = "x86_64")]`-gated only (no x86 twin): `VFS_ELF` static, `vfs` fn, `COMMANDS.insert`. The `vfs_client` app is not a `rosx-x86` dependency; `apps/shell/build.rs` existence check for `vfs_client` sits inside the existing `if arch == "x86_64" { … }` block. The `vfs_server` app is likewise never scheduled on x86_32 (ELF spawning is already commented out there). `vfs-protocol` must compile for the `x86` shell graph transitively (shell does NOT depend on it — no exposure).

---

## 2. Domain and console contracts

### 2.1 fs-layer unit-test matrix (`apps/vfs_server/src/fs.rs`, host tests)
Every numbered rule gets at least one test: open rules 1–4 (BR7); invalid names via `valid_name` (empty, 57 ok, 58 rejected, `'\n'` byte, `'\0'` byte); read empty file → EOF-empty; read at and past EOF; read clamp across end; write without WRITE → `NotPermitted`; read on mode-0 handle ok; write at 0 into empty; write appending at len; write beyond len zero-fills the hole byte-exactly (`Vec` of zeros, then payload bytes at right offsets); empty write no-op (length unchanged); close frees the handle (second close → `BadHandle`); cross-owner handle use → `BadHandle`; delete then read via still-open handle → `NotFound`; delete missing → `NotFound`; list ordering (`blob.bin` before `notes.txt`); list empty → empty; two handles on one file see shared content with independent semantics (both read the same bytes).

### 2.2 wire unit-test matrix (`vfs-protocol/src/wire.rs`, host tests)
Golden byte vectors pinning offsets and LE-ness for at least: OPEN (name `"a.txt"`), READ (handle `0x01020304`, offset `0x0807060504030201`, count `0x00010000`), WRITE, Listed — pattern: assert exact indices like `system/src/ipc.rs:33-42` does. Round-trip `encode → decode_request` for all six ops; `decode_request` rejects unknown op, `name_len = 0`, `name_len = 58`, declared name past byte 60. `decode_reply` under each `Expect` incl. wrong-shape → `Err`; Opened with handle 0 → `Err`; EOF frame shape; error-code round-trip incl. unknown byte → `VfsError::Protocol` mapping helper.

### 2.3 payload pattern (deterministic, position-based)
```rust
fn vfs_byte(file_offset: usize) -> u8 { (file_offset as u64).wrapping_mul(29).wrapping_add(11) as u8 }
```
Used by `vfs_client` for both files; the hole in `notes.txt` is asserted as all-zero (0 never collides with the pattern? pattern at any i is `i*29+11 mod 256`, which equals 0 for some i — assert the hole range `[4096, 8192)` is zero **as a range property**, and verify pattern only on `[0, 4096)` and `[8192, 9216)`; the ranges do not overlap the checked pattern segments).

### 2.4 client console grammar (exact strings, printed by `vfs_client`, in order)
Any failed check prints `vfs error: <check-name>` and returns from `_start` immediately; a step counts as passed only when the **exact expected error variant** is observed (wrong error → failure line).

```
[ELF] VFS Client
vfs ok: create notes.txt
vfs ok: write 4096 at 0
vfs ok: write 1024 at 8192
vfs ok: read 9216 hole zeroed
vfs ok: eof at 9216
vfs ok: zero write accepted
vfs ok: write denied mode 0
vfs ok: reopen kept 9216
vfs ok: list got notes.txt
vfs ok: create existing denied
vfs ok: open missing denied
vfs ok: chunked write 131072
vfs ok: chunked read 131072 verified
vfs ok: list got blob.bin notes.txt
vfs ok: delete notes.txt
vfs ok: delete twice denied
vfs ok: list got blob.bin
vfs ok: delete blob.bin
vfs ok: list empty
vfs done
```
Scenario mechanics — handle lifetimes are explicit; every `open` returns a fresh `VfsFile`, `close(self)` consumes it; `seek` is the BR10 client-side assignment (state, not RPC):
1. `open("notes.txt", MODE_WRITE | MODE_CREATE)` → Ok.
2. same handle: `write` 4096 pattern bytes at its offset 0 → Ok, offset 4096.
3. `close` it; fresh `open("notes.txt", MODE_WRITE)`, `seek(8192)`, `write` 1024 pattern → Ok; `close`.
4. fresh `open("notes.txt", 0)`: `read` exactly 9216 → Ok, pattern verified on `[0,4096)` and `[8192,9216)`, all-zero on `[4096,8192)`; handle offset now 9216.
5. same handle: `read` → `Ok(0)` (EOF at 9216); `close`.
6. fresh `open("notes.txt", MODE_WRITE)`: `write(&[])` → `Ok(())`; `close`.
7. fresh `open("notes.txt", 0)`: `write` 1 byte → `Err` with **exactly** `VfsError::NotPermitted`; `close`.
8. fresh `open("notes.txt", 0)`: `read` → exactly 9216 bytes (plain open never truncated); `close`.
9. `list()` == `["notes.txt"]`.
10. `open("notes.txt", MODE_CREATE)` → **exactly** `AlreadyExists`.
11. `open("missing.txt", 0)` → **exactly** `NotFound`.
12. `open("blob.bin", MODE_WRITE | MODE_CREATE)`: `write` 131072 pattern bytes = exactly two 64 KiB RPC chunks → Ok; `close`.
13. fresh `open("blob.bin", 0)`: `read` 131072, verify `vfs_byte(i)` at every index; `close`.
14. `list()` == `["blob.bin", "notes.txt"]`.
15. `delete("notes.txt")` → Ok.
16. `delete("notes.txt")` again → **exactly** `NotFound`.
17. `list()` == `["blob.bin"]`.
18. `delete("blob.bin")` → Ok.
19. `list()` → empty vec (server `EOF` status path).
Then `vfs.disconnect()` and `_start` **returns** (clean exit — ping-plan BR3 evidence; an idle tail would freeze the shell).

### 2.5 server console lines
Boot: `[IPC Server] VFS` as the first line of `_start` (bulk_echo shape). Panic handler: `[IPC Server] VFS - Panic!` then loop. Nothing else ever printed.

### 2.6 accepted-for-iteration-1 limitations (reproduce verbatim in the closing docs)
Volatile store (everything dies with the server); synchronous protocol, one request in flight per connection, no request ids; no client-death cleanup — leaked open entries and held reply tokens (decision 6); ≥2 copies per transfer (copy-in snapshot → fs buffer → reply object copy-out); per-RPC round-trip latency dominates, 64 KiB chunk ceiling; chunked writes are not atomic; no delete-permission model (any connected client deletes any name); zero-fill on hole writes; no `seek` on the wire (client-side only); server serializes all clients.

---

## 3. Implementation steps

### Step 1 — Create `vfs-protocol` (wire codec + typed client)
**Files:** `vfs-protocol/Cargo.toml`, `src/lib.rs`, `src/wire.rs`, `src/client.rs`; root `Cargo.toml` (insert `"vfs-protocol",` in `members` after `"usrlib",` and in `default-members` after `"usrlib",`); `Cargo.lock`.
**Contract:** modules per BR3/BR4/BR9/BR10; every public item exactly as listed; unit tests per §2.2 (`#[cfg(test)]` in `wire.rs`; `client.rs` gets no host tests — it is syscall-bound glue).
**Why first:** everything downstream compiles against these names; nothing here depends on later steps.
**Gate:** `timeout 300s cargo test -p vfs-protocol` @repo root — `unverified` form (same class as the green `cargo test -p kernel -- --test-threads=1` precedent; needs the orchestrator baseline pass to prove).
**Commit:** `Add VFS protocol crate with wire codec and client`

### Step 2 — Create `apps/vfs_server` (RamFs lib + RPC server bin)
**Files:** `apps/vfs_server/{Cargo.toml,build.rs,linker.ld,src/lib.rs,src/fs.rs,src/main.rs}`; root `Cargo.toml` members += `"apps/vfs_server",`; `xtask/src/main.rs:5` → `[&str; 10]` appending `"vfs_server"`; `Cargo.lock`.
**Contract:** package shape per BR9 (link-args gated on `CARGO_CFG_TARGET_OS == "none"`); `fs.rs` per BR6/BR7/BR8 + §2.1 tests (owner is a `u32`, no usrlib import); `main.rs` per BR11 with the accept-loop skeleton of `bulk_echo` and `println!("[IPC Server] VFS");` in `_start`; `data_token`/`conn_token` helpers per BR2; panic handler per §2.5.
**Compilability:** depends only on merged `usrlib`/`system` surface + Step 1 crate.
**Gates:** `timeout 300s cargo test -p vfs_server --lib -- --test-threads=1` @root — `unverified`; AND `timeout 900s cargo xtask apps` @root (green form, prior-run roster 9 → now builds 10 ELFs; confirm `target/rosx-user/release/vfs_server` exists afterward).
**Commit:** `Add VFS server with in-memory file store`

### Step 3 — Schedule `vfs_server` at x86_64 boot
**Files:** `arch/x86_64/src/main.rs`: insert immediately **before** the Shell schedule line (`FunctionTask::new("Shell", shell::shell::main)`) the pair `static VFS_SERVER_ELF: &[u8] = include_bytes!("../../../target/rosx-user/release/vfs_server"); let _ = kernel.schedule(new_elf_task(VFS_SERVER_ELF));` — after `PING_SERVER_ELF`, mirroring the existing block style verbatim. `arch/x86_64/build.rs`: append the `vfs_server` `rerun-if-changed` triplet and an existence panic, mirroring the ping_server entries verbatim.
**Why before shell:** the `[IPC Server] VFS` bind must precede the prompt; the client's bounded retry (BR10) is the backstop.
**Gate:** `timeout 900s cargo xtask build` @root (green form: apps → kernel → disk image; exercises the new `include_bytes!`).
**Commit:** `Schedule VFS server at x86_64 boot`

### Step 4 — `vfs_client` app + shell `vfs` command
**Files:** `apps/vfs_client/{Cargo.toml,build.rs,linker.ld,src/main.rs}` (verbatim `bulk_echo` skeleton + `vfs-protocol` dep, names `vfs_client`); root `Cargo.toml` members += `"apps/vfs_client",`; `xtask/src/main.rs:5` → `[&str; 11]` appending `"vfs_client"`; `apps/shell/src/shell.rs` + `apps/shell/build.rs`; `Cargo.lock`.
**Contract:** `vfs_client::_start` = scenario §2.4 exactly (check names in error lines = the `vfs error:` strings), uses only the BR10 client API + `println!`; returns after `disconnect`. Shell: `#[cfg(target_arch = "x86_64")] static VFS_ELF = include_bytes!("../../../target/rosx-user/release/vfs_client");` after `PING_ELF` (`:31`); cfg-gated `fn vfs()` in the `Syscall::load` + `wait_future` form (`:140-145` verbatim shape); `commands.insert(String::from("vfs"), vfs as fn());` inside the existing block-form `COMMANDS` initializer; `apps/shell/build.rs` gains the `vfs_client` rerun triplet (unconditional, like `ping_client`) and the `vfs_client` existence check inside the existing `if arch == "x86_64"` block.
**Blast radius (shared global registry):** `COMMANDS` is consumed by `ls` and dispatch — every QEMU session that touches the prompt (`boot_and_shell` ×4, `app_lifecycle` ×2, `ipc_random`, `ipc_bulk`, `ipc_ping`, new `ipc_vfs`). `vfs` sorts between `tests` and `tetris` in `ls`; additive insert verified neutral against the needle set of `boot_and_shell.rs` (asserts `"snake"`, `"tetris"`, `"rose>"`); full behavioral proof is Step 5, not the build gate.
**Gates:** `timeout 900s cargo xtask apps` @root (11 ELFs); `timeout 900s cargo xtask build` @root; `timeout 900s cargo build -p rosx-x86` @`arch/x86_32/` (green form; proves zero new x86 preconditions — neither vfs app is a `rosx-x86` dependency).
**Commit:** `Add VFS client app and shell vfs command`

### Step 5 — Integration test `ipc_vfs`
**File:** `tests-integration/tests/ipc_vfs.rs` (template `ipc_ping.rs`; helpers retargeted). No production file changes.
**Contract:**
```
const READY_TIMEOUT: Duration = Duration::from_secs(60);
const SWEEP_TIMEOUT: Duration = Duration::from_secs(180);
const EXPECTED_OK: [&str; 19] = [ the §2.4 lines between the boot and done lines, in order ];
#[test] fn shell_vfs_filesystem_roundtrip()
```
Flow: `let mut session = QemuSession::spawn();` → `let mark = session.mark();` first → `expect_output("[IPC Server] VFS", READY_TIMEOUT)` → `expect_output("rose>", READY_TIMEOUT)` → `send_line("vfs")` → `wait_until_since(mark, |d| d.contains("vfs done"), SWEEP_TIMEOUT)` (panic-with-tail on `Err`, `ipc_bulk` shape) → `content_since(mark)` then: lines starting `"vfs ok: "` number exactly 19 and equal `EXPECTED_OK` element-wise in order; exactly one line equals `"vfs done"` and it follows the last ok line; no line starts with `"vfs error:"`; content contains neither `"fail"` nor `"Panic"`. All failures embed the 2000-char tail window (convention).
**Non-assumptions:** the `send_line("vfs")` echo lands in the window — `"vfs"` alone matches none of the full prefixes; the boot-time bulk exchange (~t+40 s) and any ping output may interleave — matched prefixes are exclusive of `BULK `/`ping sent: `/`PONG `; green bulk/ping output is `fail`/`Panic`-free per the ping-plan risk-6 sweep. Do not assert console silence.
**Gate:** `timeout 1200s cargo xtask test` @root (green form) — expected: all host suites green (new host tests = whatever §2.1/§2.2 add, verifier counts from the diff; zero failures anywhere) and QEMU roster **10** (app_lifecycle 2, boot_and_shell 4, ipc_bulk 1, ipc_random 1, ipc_ping 1, ipc_vfs 1).
**Commit:** `Add QEMU integration test for VFS roundtrip`

### Step 6 — AGENTS.md structure-tree update
**File:** `AGENTS.md`: add `vfs-protocol` to the crate rows beside `system`/`usrlib` and append `vfs_server, vfs_client` to the `[workspace members]` apps list (current line verbatim anchor: `shell, dummy, hello_elf, random_gen_server, bulk_echo, bulk_client, ping_server, ping_client, snake, tetris, conway, test_suite`). No prose beyond the tree; the limitations text (§2.6) already lives in this plan document.
**Gate:** none (documentation).
**Commit:** `List VFS protocol and apps in AGENTS.md structure tree`

---

## 4. Risks and edge cases (coder must handle / verifier must know)

1. **Host linkability of `vfs_server` (the novel risk).** The lib's host test harness links against the package's `build.rs` link-args — unguarded they would inject the bare-metal `linker.ld` into a host binary. BR9's `CARGO_CFG_TARGET_OS == "none"` gate is the fix; the Step 2 `--lib` test gate is its falsifiable probe (`unverified` until green).
2. **`decode_reply` ambiguity.** OK-status replies share byte ranges across Opened/ReadData/Written/Listed; the `Expect` parameter is the discriminator. A client that passes the wrong Expect must fail loudly (`Protocol`), never silently misread — pinned by §2.2 tests.
3. **Chunked-write partiality.** A multi-chunk `write` that fails at chunk *k* leaves *k−1* chunks applied; the reply is `Err` and the offset reflects only acknowledged bytes. Documented (§2.6); `vfs_client` never depends on failure mid-chunk (single client, healthy server).
4. **Server budget.** Server is the sole writer of READ/LIST replies (256 KiB per task): held-reply-per-conn (BR5) + one current object keeps scenario peak at 64 KiB + 64 KiB. Overflow path exists and is wired: `ERR(ServerResource)`; unreachable in the single-client scenario but unit-covered only at fs level — a full kernel-budget path is intentionally not integration-tested this iteration.
5. **Client hangs if the server diverges.** No client-side RPC timeouts by design (usrlib `wait_for_message` blocks); a protocol bug surfaces as the `ipc_vfs` SWEEP_TIMEOUT with an output tail — same failure mode as `ipc_ping`.
6. **Empty-frame edges.** Empty READ count → `EOF` before fs; zero-length WRITE payload → fs no-op but the kernel data object is still created and released (uniform path, no wire sentinel; empty `write_data` is accepted by the kernel, §0). `list()` on empty store → `EOF` with no object; `Listed` always carries count ≥ 1.
7. **Keystroke echo & interleaving.** §2.5 non-assumptions; every assertion is full-prefix or whole-line equality.
8. **New boot line `[IPC Server] VFS`** appears in every future session; checked against every `contains` needle in `boot_and_shell.rs`, `app_lifecycle.rs`, `ipc_random.rs`, `ipc_bulk.rs`, `ipc_ping.rs` — no collision either direction; forbidden strings unaffected (line is `Panic`-free in green operation).
9. **`ls` registry growth** (`vfs` key): additive; `boot_and_shell` needle set verified neutral (same analysis as `ping` in the prior plan).
10. **CI blindness.** `ci.yml` never runs `tests-integration`; `ipc_vfs` guards local `cargo xtask test` only — same protection level as `ipc_ping`/`ipc_bulk`.
11. **Host `cargo test --workspace` builds `vfs-protocol` for x86_64-linux** — usrlib's `syscall`-insn shim compiles on host and is never invoked by unit tests; the `fs`/`wire` test modules have zero syscall reach by the BR9 module-dependency law.

---

## 5. Acceptance criteria (exact commands)

| # | Command | CWD | Expected |
|---|---|---|---|
| 1 | `timeout 300s cargo test -p vfs-protocol` | repo root | all wire tests pass — `unverified` form, proven at baseline |
| 2 | `timeout 300s cargo test -p vfs_server --lib -- --test-threads=1` | repo root | all fs tests pass — `unverified` form, proven at baseline |
| 3 | `timeout 900s cargo xtask apps` | repo root | 11 ELFs in `target/rosx-user/release/` incl. `vfs_server`, `vfs_client` |
| 4 | `timeout 900s cargo xtask build` | repo root | reaches `==> create x86_64 disk image`; `^error\|error\[` grep = 0 |
| 5 | `timeout 900s cargo build -p rosx-x86` | `arch/x86_32/` | exit 0; no `vfs` mention in the log (not a dependency) |
| 6 | `timeout 1200s cargo xtask test` | repo root | all suites green; QEMU roster = 10 incl. `ipc_vfs::shell_vfs_filesystem_roundtrip ... ok`; host totals = baseline + the new §2.1/§2.2 tests, 0 failures |

**Warning gate — Δ-new = 0** against the recorded branch fingerprint (`.opencode/artifacts/ipc-ping-verify-final.md`: kernel lib 60; kernel lib-test 83 (49 dup); shell lib 3; rosx bin 26; rosx manifest 1; rosx-x86 bin 1; tests-integration lib 1), arbitrated **location-first** (normalized `warning text|file` pairs). `apps/shell/src/shell.rs` is touched → line shifts are expected; the shell trio must still be exactly 3 sites with byte-identical content, shift-attributed; new artifacts (`vfs-protocol`, both vfs apps, `ipc_vfs`, `xtask/`, `arch/x86_64/`) contribute **zero** unique warnings (`--> vfs` grep = 0). Counting filter matches both `warning:` and `warning[EXXXX]:`. Totals corroborate, never convict; decreases attributed by baseline multiset diff.

**Comment law gate (portable POSIX grep):**
`grep -nE '(^|[[:space:]])//[^!]' vfs-protocol/src/*.rs apps/vfs_server/src/*.rs apps/vfs_client/src/main.rs tests-integration/tests/ipc_vfs.rs` → no output; `git diff` shows zero added lines containing `//` or `/*` in `apps/shell/src/shell.rs`, `apps/shell/build.rs`, `arch/x86_64/src/main.rs`, `arch/x86_64/build.rs`, `xtask/src/main.rs`, root `Cargo.toml`.

**Diff-boundary gate:** `git diff --name-only <step0-baseline>..HEAD` contains no path under `kernel/`, `system/`, `usrlib/`, `arch/x86_32/`, `arch/x86_64-runner/`, and no pre-existing app directory; the only edited pre-existing files are exactly those named by steps 1–6.

**Service-name gate:** `grep -rn "ipc_bind" apps/vfs_server/src/main.rs` shows exactly one bind, for `"VFS"`; `grep -rn "ipc_connect" vfs-protocol/src/client.rs` targets `"VFS"` only.
