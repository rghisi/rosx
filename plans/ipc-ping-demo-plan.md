# IPC Ping Demo — Implementation Plan (`ping_server` + `ping_client` + shell `ping` + `ipc_ping`)

> **Date:** 2026-10-16. **Branch:** `ipc-bulk-data-transfers` @ `1d26b76` (PR #64 open).
> **Scope:** apps + wiring + one integration test. `kernel/`, `system/`, `usrlib/`, `bulk_echo`, `bulk_client`, `ipc_bulk` receive **ZERO changes** (user-locked decision 7).
> **Baseline evidence:** the four gate forms carry green transcripts from this session's delegation pass, corroborated by `.opencode/artifacts/ipc-bulk-verify-final.md` (branch-fingerprint warning table). This plan's own claims are NOT evidence; only the acceptance run is.
> **Repo law applied:** no comments in any new Rust file (AGENTS.md); commit subjects carry no prefix; work one commit per step.

---

## 0. Verified-baseline facts (re-verified this session against HEAD 1d26b76, not the stale Context Map)

- Syscalls merged: `system/src/syscall_numbers.rs:21-23` — `IpcDataWrite = 24, IpcDataRead = 25, IpcDataRelease = 26`, mirrored in the `TryFrom<usize>` match (`:48-50`).
- usrlib surface merged and **public**: `Syscall::ipc_data_write` (`usrlib/src/syscall.rs:161-175`), `Syscall::ipc_data_read` (`:177-189`), `usrlib::ipc::receive_data` (`usrlib/src/ipc.rs:114-125`, decodes token at offset 0 and reads to EOF), `usrlib::ipc::ReceivedMessage::{connection, data}` (`:19-25`), `UserDataHandle` (`usrlib/src/handle.rs:100-158`).
- **Codec gate:** `encode_data_token`/`decode_data_token` are `pub(crate)` (`usrlib/src/ipc.rs:67-75`): token = first 4 bytes of the 60-byte `Message` payload, `u32::to_ne_bytes`. Unit test `data_token_occupies_only_the_first_four_payload_bytes` (`:139-144`) freezes offset/length/endianness.
- **`UserDataHandle` token visibility:** `packed()` is `pub(crate)` (`handle.rs:105`); the only public channel carrying the token value is `Display`/`Debug` rendering `"{:08x}"` of the packed u32 (`handle.rs:142-149`). `release()` guards with `self.packed.take()` → explicit-then-Drop is a documented no-op (`handle.rs:122-140`).
- Kernel limits (`kernel/src/ipc/data_manager.rs:8-9`): `const MAX_DATA_BYTES: usize = 64 * 1024;` and `const DATA_BUDGET_BYTES: usize = 256 * 1024;`. Boundary is strict `>` (`:40`) — exactly 65536 is accepted (unit-proven at `:206-215`). Budget is **per writer TaskHandle** (`:43-46`, unit `budget_is_per_task` `:234-245`). Disconnect-GC refunds per `DataObject.writer` (`gc_connection` `:98-114`).
- Task exit semantics (decides the client tail, see BR3): `elf_task_wrapper` (`kernel/src/task.rs:229-241`) calls `kernel().terminate_and_yield()` after `_start` returns; `terminate_and_yield` (`kernel.rs:262-277`) sets `Terminated` and calls `future_registry.notify(completion_handle)`; `TaskCompletionFuture::is_completed` is task-state `Terminated` (`future.rs:67-70`); the shell's `WaitFuture` syscall wakes on that notify (`kernel.rs:224-232`). Production precedent: `hello_elf`'s `_start` returns (`apps/hello_elf/src/main.rs:112`) behind the `pi` command (`apps/shell/src/shell.rs:107-111`).
- `xtask/src/main.rs:5` verbatim: `const USER_APPS: [&str; 7] = ["hello_elf", "random_gen_server", "snake", "tetris", "conway", "bulk_echo", "bulk_client"];` — `build_apps()` loops it `--release` for `arch/x86_64/rosx-user.json` with the 3 `-Z` flags; `run_tests()` calls `build_apps()` first (`:76`).
- Boot embedding verbatim (`arch/x86_64/src/main.rs:76-80`):
  ```rust
  static BULK_ECHO_ELF: &[u8] = include_bytes!("../../../target/rosx-user/release/bulk_echo");
  let _ = kernel.schedule(new_elf_task(BULK_ECHO_ELF));
  static BULK_CLIENT_ELF: &[u8] = include_bytes!("../../../target/rosx-user/release/bulk_client");
  let _ = kernel.schedule(new_elf_task(BULK_CLIENT_ELF));
  let _ = kernel.schedule(FunctionTask::new("Shell", shell::shell::main));
  ```
  `arch/x86_64/build.rs` carries per-app `rerun-if-changed` triplets (`:7-12`) and per-ELF existence panics (`:18-25`).
- `apps/shell/build.rs`: rerun-if-changed loop over `["hello_elf", "snake", "tetris", "conway"]` (`:4-8`); arch-selected base `"x86_64" => "../../target/rosx-user/release"`, `"x86" => "../../target/rosx-i686-user/release"` (`:10-15`); existence loop over the same four (`:17-22`).
- `apps/shell/src/shell.rs`: cfg-paired `include_bytes!` at `:10-28`; registry `lazy_static! { static ref COMMANDS: BTreeMap<String, fn()> = BTreeMap::from([...]); }` at `:32-45`; spawn pattern (`snake`, `:113-117`): `if let Some(fh) = Syscall::load(SNAKE_ELF) { Syscall::wait_future(fh); }`.
- `ls` enumerates `COMMANDS` keys (`shell.rs:102-105`); `boot_and_shell.rs::shell_ls_and_unknown_command` asserts substrings `"snake"`, `"tetris"`, `"rose>"` — additive keys are neutral (verified by inspection: no substring collision with `ping`).
- Harness API (`tests-integration/src/qemu.rs`): `spawn() :29-31`, `expect_output(&self, &str, Duration) :114`, `mark(&self) -> usize :118`, `wait_until_since(&self, mark, pred, timeout) -> Result<usize, String> :126-133`, `content_since(&self, mark) -> String` (ANSI-stripped) `:135-137`, `send_line(&mut self, &str) :175-178`. Templates: `ipc_random.rs` (send_line-driven), `ipc_bulk.rs` (mark-before-expect boot-marker style).
- Current QEMU roster = 8 (app_lifecycle 2, boot_and_shell 4, ipc_bulk 1, ipc_random 1); CI (`ci.yml`) runs only `cargo xtask test --skip-integration` + `cargo xtask build` — `tests-integration` never runs in CI (same guard level as `ipc_bulk`).
- Warning fingerprint recorded for this branch (`ipc-bulk-verify-final.md`): kernel lib **60**; kernel lib-test **83 (49 dup)** (original fingerprint 86/52 — dup-suppressed reduction, unique-per-stream unchanged); shell lib **3** (`shell.rs:132/152/166`); rosx bin **26** + rosx manifest **1** (`unused dependency dummy`); rosx-x86 bin **1** (`unused_unsafe`, `arch/x86_32/src/main.rs:66:22`); tests-integration lib **1**.
- App skeleton to clone verbatim: `apps/bulk_echo/{Cargo.toml,build.rs,linker.ld}` + `apps/bulk_echo/src/main.rs` shape (`#![no_std] #![no_main]`, `extern crate alloc;`, `SyscallAllocator`, `#[unsafe(no_mangle)] pub extern "C" fn _start`, own `#[panic_handler]`).

---

## 1. Binding resolutions (the coder follows these verbatim)

**BR1 — "spawn via `Syscall::exec` of an `include_bytes!`ed ELF, the snake pattern" resolved.** `Syscall::exec` takes an in-process entrypoint `usize` (`usrlib/src/syscall.rs:14-17`; its only shell use is `test_suite::app::main as usize`, `shell.rs:132`) and cannot load an ELF. The snake pattern — the named reference — is `Syscall::load(&'static [u8])` (LoadElf, syscall 10) + `Syscall::wait_future(fh)` (`shell.rs:113-117`). The `ping` command implements exactly that form.

**BR2 — data token extraction with zero usrlib changes.** `UserDataHandle::packed()` is `pub(crate)`, so an app cannot read the token numerically. Its public `Display` renders `"{:08x}"` of the packed u32 (`handle.rs:142-149`), and packed values are always `< ERROR_BIT` (kernel `handle.rs:pack()` never sets bit 31 — exhaustively swept in kernel unit tests), so `Display` output is always exactly 8 lowercase hex digits. Each new app defines a private:
```rust
fn data_token(handle: &UserDataHandle) -> u32 {
    u32::from_str_radix(&format!("{}", handle), 16).expect("canonical data token")
}
```
This round-trips byte-exactly to what `encode_data_token` writes with `to_ne_bytes` at offset 0 (`usrlib/src/ipc.rs:67-69`) — hand-assembled canonical encoding, no usrlib change. The `expect` is unreachable while the handle is unreleased (encoding always precedes release in both flows); a panic there is caught by the integration forbidden-string set. Requires `use alloc::format;` (edition 2024).

**BR3 — client tail: clean task exit (chosen on evidence, infinite-sleep tail rejected).** A returning `_start` is fully supported: `elf_task_wrapper` → `terminate_and_yield` → `Terminated` + `future_registry.notify(completion_handle)` → shell's `wait_future` wakes (evidence in §0; production precedent `hello_elf` via `pi`). A bulk_client-style `loop { Syscall::sleep(1000); }` tail is **incompatible with the snake pattern** — `wait_future` would never return and the shell would freeze. Therefore `ping_client::_start` runs its 3 rounds, disconnects, and **returns**. Repeated `ping` invocations are inherently sequential (the shell blocks in `wait_future`), so no concurrent clients exist; each invocation gets a fresh connection and fresh handles.

**BR4 — pong payload size equals ping payload size** ("same size", decision 2). Consequently every client line prints the same size twice: `ping sent: 4096 ok received: 4096 ok`.

**BR5 — release choreography (no protocol change, no reader release anywhere).**
- Client (writer of PING): after printing the round's verdict line, `let _ = ping_handle.release();` — the explicit release; the subsequent `Drop` is a no-op via `packed.take()` (`handle.rs:123`).
- Server (writer of PONG): holds `Option<UserDataHandle>` across the accept loop; at the **start of each round** `if let Some(mut held) = previous_pong.take() { let _ = held.release(); }` — next-PING implicit ack.
- The last PONG of the final round is outstanding at `disconnect` and is reclaimed by kernel disconnect-GC (`data_manager.rs:98-114`). If the server later attempts `release()` on that GC'd token (only possible if a future PING arrives after a client disconnect), the kernel answers `DataError::NotFound` and the `let _ =` discards it — correct no-op.
- Readers never hold a handle: `receive_data` returns a `Vec<u8>`. Nothing to release.
- Budget safety: peak outstanding per writer task is 64 KiB (client holds at most one unreleased PING; server at most one unreleased PONG) against the 256 KiB per-task budget; the bulk pair runs under different TaskHandles and budgets never cross-debit (`budget_is_per_task`).

**BR6 — x86_32 stays precondition-free.** Every artifact the client side introduces is `#[cfg(target_arch = "x86_64")]`-gated ONLY (no `target_arch = "x86"` twin): the `PING_ELF` static, the `ping` fn, and the `COMMANDS` insert. `apps/shell/build.rs` gates the ping existence check on the runtime `arch == "x86_64"` branch. The server is scheduled only from `arch/x86_64/src/main.rs`. Neither new app is a dependency of `rosx-x86`, so gate 4 builds neither.

**BR7 — untouched-file law.** `kernel/`, `system/`, `usrlib/`, `apps/bulk_echo/`, `apps/bulk_client/`, `tests-integration/tests/ipc_bulk.rs` must not appear in any diff of this work. (`xtask/src/main.rs` is wiring: only the `USER_APPS` line grows; the 7 existing entries stay verbatim, including both bulk apps.)

**BR8 — no comments, ever**, in any new or edited Rust line (AGENTS.md). Self-explanatory names only.

**BR9 — `COMMANDS` registry extension shape.** Stable Rust does not allow `#[cfg]` on an element inside a `BTreeMap::from([...])` array literal. Wrap the existing lazy_static initializer in a block expression: the existing `BTreeMap::from([...])` literal stays byte-identical, then a cfg'd insert, then return the map:
```rust
lazy_static! {
    static ref COMMANDS: BTreeMap<String, fn()> = {
        let mut commands: BTreeMap<String, fn()> = BTreeMap::from([ /* existing 10 entries, verbatim */ ]);
        #[cfg(target_arch = "x86_64")]
        commands.insert(String::from("ping"), ping as fn());
        commands
    };
}
```
`BTreeMap::insert` is not `#[must_use]` — no new warning. `ping` sorts between `pi` and `random` in `ls` output; additive and assertion-neutral for `boot_and_shell` (verified against its needle set).

---

## 2. Protocol and console grammar (verbatim contract)

### 2.1 Wire message (both directions): 60-byte `Message` payload
- Bytes `[0..4]`: data token, `u32::to_ne_bytes` (canonical; byte-identical to `encode_data_token`, `usrlib/src/ipc.rs:67-69`), produced per **BR2**.
- Bytes `[4..4+L]`: ASCII text below; all remaining bytes zero.
- Client→server text: `PING:<size>:<checksum>` — `<size>` decimal payload length; `<checksum>` lowercase 8-digit hex (`{:08x}`) of the wrapping-u32 byte-sum checksum of the PING payload. Example: `PING:65536:0009e000`.
- Server→client text: `PONG:<matched>:<size>:<checksum>` — `<matched>` ∈ `ok|fail` = server verdict (received checksum == announced checksum); `<size>`/`<checksum>` describe the PONG payload. Max length `PONG:ok:65536:ffffffff` = 22 chars; 4+22 = 26 ≤ 60.

### 2.2 Payload patterns (deterministic)
```rust
fn ping_byte(index: usize) -> u8 { (index as u32).wrapping_mul(31).wrapping_add(17) as u8 }
fn pong_byte(index: usize) -> u8 { (index as u32).wrapping_mul(37).wrapping_add(23) as u8 }
```
Checksum function in BOTH apps, identical to `bulk_echo`'s (`apps/bulk_echo/src/main.rs:27-31`):
```rust
fn checksum(payload: &[u8]) -> u32 {
    payload
        .iter()
        .fold(0u32, |sum, byte| sum.wrapping_add(*byte as u32))
}
```

### 2.3 Console lines (exact strings)
| Producer | Line | Where |
|---|---|---|
| ping_server boot | `[IPC Server] Ping` | first line of `_start`, before the bind loop (bulk_echo shape, `bulk_echo/src/main.rs:58-63`) |
| ping_server per round | `PONG <size> matched ok` / `PONG <size> matched fail` | `println!("PONG {} matched {}", pong_len, verdict_word)` |
| ping_client per round | `ping sent: <size> <ok\|fail> received: <size> <ok\|fail>` | `println!("ping sent: {} {} received: {} {}", sent_size, sent_verdict, received_len, recv_verdict)` — EXACTLY one line per round |
| ping_server panic handler | `[IPC Server] Ping - Panic!` | mirrors `bulk_echo/src/main.rs:65-71` |
| ping_client panic handler | `[PING Client] Panic!` | mirrors `bulk_client/src/main.rs:127-133` |

### 2.4 Client verdict rules
- `sent_verdict` = `ok` iff the PONG text's `matched` field is `ok` (server-validated first verdict).
- `received_verdict` = `ok` iff `checksum(&pong_bytes) ==` the checksum announced in the PONG text (recomputed second verdict). `received: <size>` prints the measured `pong_bytes.len()`.
- Failure paths print exactly one line and proceed: write/send error → `ping sent: <size> fail received: 0 fail`; PONG bytes received but text unreadable → `ping sent: <size> fail received: <len> fail`.

### 2.5 Round sizes
`const PING_SIZES: [usize; 3] = [64, 4096, 65536];` — 65536 is exactly `MAX_DATA_BYTES` (boundary probe; accepted because the kernel check is strict `>`, `data_manager.rs:40`).

---

## 3. Implementation steps

### Step 1 — Create both apps; register in workspace and xtask
**Files:**
- Create `apps/ping_server/Cargo.toml`, `build.rs`, `linker.ld` — verbatim clones of `apps/bulk_echo/`'s, with `name = "ping_server"` (both in `[package]` and `[[bin]]`).
- Create `apps/ping_client/{Cargo.toml,build.rs,linker.ld}` — same clone with `name = "ping_client"`.
- Create `apps/ping_server/src/main.rs` — contract: `#![no_std] #![no_main]`, `extern crate alloc;`, `SyscallAllocator` clone (`bulk_echo/src/main.rs:12-25`), imports limited to what is used (`alloc::vec::Vec`, `alloc::format`, `core::alloc::{GlobalAlloc, Layout}`, `core::panic::PanicInfo`, `usrlib::println`, `usrlib::syscall::Syscall`, `usrlib::handle::UserDataHandle`, `usrlib::ipc::receive_data`, `system::ipc::MESSAGE_PAYLOAD_BYTES`). `checksum` per §2.2, `pong_byte` per §2.2, `data_token` per BR2. `_start`: `println!("[IPC Server] Ping");` then run loop. Run loop = bulk_echo's accept loop (`bulk_echo/src/main.rs:36-46`, bind `"PING"`) plus a `let mut previous_pong: Option<UserDataHandle> = None;` threaded through; per round, in order:
  1. BR5 ack-release of `previous_pong` (`take()` + `let _ = held.release();`).
  2. `receive_data(&received)`; on `Err` → continue (no reply; unreachable on green).
  3. Parse announced checksum: `core::str::from_utf8(&received.data()[4..])`, `trim_end_matches('\0')`, `split(':')`, third field `u32::from_str_radix(_, 16)`; unparseable ⇒ `None`.
  4. `matched = (announced == Some(checksum(&ping_payload)))`.
  5. `pong_payload: Vec<u8> = (0..ping_payload.len()).map(pong_byte).collect();` (BR4: same length).
  6. `Syscall::ipc_data_write(connection, &pong_payload)` with `connection = received.connection()` (guard `if let Some`, bulk_echo `:51`); on `Err` → continue.
  7. Build reply `data = [0u8; MESSAGE_PAYLOAD_BYTES]`; `data[..4].copy_from_slice(&data_token(&pong_handle).to_ne_bytes());` then text `format!("PONG:{}:{:08x}", verdict_word, pong_payload.len(), checksum(&pong_payload))` with `verdict_word` `"ok"`/`"fail"`; `Syscall::ipc_send(connection, &data)`.
  8. `println!("PONG {} matched {}", pong_payload.len(), verdict_word);`
  9. `previous_pong = Some(pong_handle);`
  Panic handler prints `[IPC Server] Ping - Panic!` then loops.
- Create `apps/ping_client/src/main.rs` — same skeleton (`bulk_client` imports style), `const MAX_DATA_BYTES: usize = 64 * 1024;` clone (`bulk_client/src/main.rs:31`), `PING_SIZES` per §2.5, `checksum` §2.2, `ping_byte` §2.2, `data_token` BR2. Connection acquisition = silent retry loop identical in shape to `connect_to_bulk_echo` (`bulk_client/src/main.rs:40-47`) but connecting `"PING"`. Per round (sizes in order): build payload `ping_byte`; `Syscall::ipc_data_write` → handle; assemble token+text message per §2.1 (`PING:{size}:{checksum:08x}`) and `Syscall::ipc_send`; wait reply via `Syscall::ipc_receive_message(connection)` + `Syscall::wait_for_message(fh)`; `usrlib::ipc::receive_data(&pong)`; parse PONG text fields (`matched`, size, hex checksum) from `pong.data()[4..]`; verdicts per §2.4; print the one §2.3 line; then BR5 explicit `let _ = ping_handle.release();`. After the loop: `Syscall::ipc_disconnect(connection);` then `_start` returns (BR3 — no sleep tail, no trailing print). Panic handler prints `[PING Client] Panic!` then loops.
- Edit root `Cargo.toml`: insert `"apps/ping_server",` and `"apps/ping_client",` after `"apps/bulk_client",` (currently line 13).
- Edit `xtask/src/main.rs:5`: `const USER_APPS: [&str; 9] = ["hello_elf", "random_gen_server", "snake", "tetris", "conway", "bulk_echo", "bulk_client", "ping_server", "ping_client"];`
- `Cargo.lock` updates automatically — commit it here.

**Why first:** everything downstream (boot `include_bytes!`, shell `include_bytes!`, image build, integration test) needs these ELFs in `target/rosx-user/release/`; no step may reference the other two before they exist.
**Compilability:** new crates depend only on `system` + `usrlib` public surface (all merged); no existing crate's source changes affect any other target. Kernel/shell/x86 builds untouched.
**Gate:** `timeout 900s cargo xtask apps` @root (proven green this session; extended list → builds 9 ELFs). Confirm `target/rosx-user/release/ping_server` and `target/rosx-user/release/ping_client` exist afterward.
**Commit:** `Add ping demo server and client applications`

### Step 2 — Schedule `ping_server` at x86_64 boot
**Files:**
- `arch/x86_64/src/main.rs`: insert after the BULK_CLIENT schedule (`:79`) and before the Shell schedule (`:80`):
  ```rust
  static PING_SERVER_ELF: &[u8] = include_bytes!("../../../target/rosx-user/release/ping_server");
  let _ = kernel.schedule(new_elf_task(PING_SERVER_ELF));
  ```
- `arch/x86_64/build.rs`: append rerun-if-changed triplet for `../../apps/ping_server` (`src`, `Cargo.toml`, `linker.ld`) mirroring `:10-12`, and an existence panic mirroring `:22-25` for `"../../target/rosx-user/release/ping_server"`.

**Why:** decision 1/5 — kernel-scheduled x86_64-only, exactly the bulk-pair pattern. Placed before `Shell` so `[IPC Server] Ping` is scheduled by the time the prompt appears.
**Compilability:** ELF exists from Step 1's gate; the build.rs panic is the backstop. No other crate sees this.
**Gate:** `timeout 900s cargo xtask build` @root (proven green; exercises apps→kernel→image with the new boot embedding).
**Commit:** `Schedule ping server at x86_64 boot`

### Step 3 — Shell `ping` command (x86_64-only)
**Files:**
- `apps/shell/src/shell.rs`:
  - Add after `CONWAY_ELF` (`:25-28`), **x86_64 gate only, no x86 twin** (BR6):
    ```rust
    #[cfg(target_arch = "x86_64")]
    static PING_ELF: &[u8] = include_bytes!("../../../target/rosx-user/release/ping_client");
    ```
  - Add the command fn, snake form verbatim per BR1, `#[cfg(target_arch = "x86_64")]`:
    ```rust
    fn ping() {
        if let Some(fh) = Syscall::load(PING_ELF) {
            Syscall::wait_future(fh);
        }
    }
    ```
  - Registry: apply the BR9 block-form lazy_static with the cfg'd `insert(String::from("ping"), ping as fn())`; the ten existing `(name, fn)` entries stay byte-identical.
- `apps/shell/build.rs`:
  - Add `rerun-if-changed` triplet for `../../apps/ping_client` (`src`, `Cargo.toml`, `linker.ld`) — unconditional is fine: the path exists for every target build of the shell and is harmless on x86.
  - After the existing existence loop (`:17-22`), add an x86_64-only existence check (runtime, not cfg): `if arch == "x86_64" { let ping_path = format!("{base}/ping_client"); if !Path::new(&ping_path).exists() { panic!("missing user ELF {ping_path}; run `cargo xtask apps` first"); } }` — the x86 branch gains no precondition (BR6).

**Why:** decision 1/5. The command can only land after `target/rosx-user/release/ping_client` exists (Step 1), or the x86_64 shell build breaks on `include_bytes!` and the shell build.rs panic — order honored.
**Compilability:** x86_64 host/bare builds see static + fn + insert; x86 (rosx-x86 → shell) sees none of them (all `#[cfg(target_arch = "x86_64")]`); host `cargo test --workspace` builds the shell lib for x86_64-linux where the cfg is true — same requirement class as snake/tetris today (ELF present because `xtask test` runs `build_apps()` first).
**Blast radius (shared global registry):** `COMMANDS` is consumed by `ls` and the dispatch loop; transitively exercised by every QEMU session that types at the prompt — `boot_and_shell` (`shell_banner_and_prompt`, `shell_echoes_keystrokes`, `shell_ls_and_unknown_command`), `ipc_random`, `ipc_bulk`, `app_lifecycle` (2), and the new `ipc_ping`. The shell's 5 host unit tests live in `command.rs` and test `Command::parse` only (no registry reach). Additive insert verified assertion-neutral against every needle in those tests (no substring collision; `ls` remains tab-joined with `ping` between `pi` and `random`). Full behavioral proof is the Step 4 acceptance suite run — a build gate alone does not discharge this row.
**Gates:** `timeout 900s cargo xtask build` @root (proven) AND `timeout 900s cargo build -p rosx-x86` @`arch/x86_32/` (proven; proves decision 5's "gate 4 gains zero new preconditions" — no i686 `ping_client` required).
**Commit:** `Add ping shell command spawning ping client`

### Step 4 — Integration test `ipc_ping`
**File:** create `tests-integration/tests/ipc_ping.rs` (template: `ipc_bulk.rs` boot/mark style + `ipc_random.rs` send_line drive; helpers `count_prefixes`/`tail_chars`/`window_diagnostics` cloned and retargeted — `ipc_bulk.rs:55-78`). No production file changes.

**Contract:**
```
const READY_TIMEOUT: Duration = Duration::from_secs(60);
const SWEEP_TIMEOUT: Duration = Duration::from_secs(180);
const PING_SIZES: [usize; 3] = [64, 4096, 65536];
#[test] fn shell_ping_capability_data_roundtrip()
```
Flow: `let mut session = QemuSession::spawn();` → `let mark = session.mark();` **first** → `session.expect_output("[IPC Server] Ping", READY_TIMEOUT);` → `session.expect_output("rose>", READY_TIMEOUT);` → `session.send_line("ping");` → `session.wait_until_since(mark, |delta| delta.contains("ping sent: 65536"), SWEEP_TIMEOUT)`, panicking with the output tail on `Err` (ipc_bulk `:88-94` shape). Then `let content = session.content_since(mark);` and assert:
1. Client lines: lines `starts_with("ping sent: ")` number exactly 3; each splits to exactly 7 whitespace tokens of shape `ping sent: <size> <verdict> received: <size> <verdict>`; the size pair on line *k* equals `PING_SIZES[k]` on both sides (in order 64, 4096, 65536); all four verdict tokens are `ok`.
2. Server lines: lines `starts_with("PONG ")` and containing `" matched "` number exactly 3; parsed sizes are 64, 4096, 65536 in order; every verdict field is `ok`.
3. Forbidden: `content` contains neither `"fail"` nor `"Panic"` (ipc_random `:153-159` loop shape).
All assertion failures embed `window_diagnostics(&content)` with the 2000-char tail (existing convention).
**Non-assumptions baked in:** the typed keystroke echo ("ping" + newline) lands in the window — full-prefix matching makes it immune; the boot-time bulk exchange fires at ~t+40 s in this session too — the window may legitimately contain `BULK *` lines, which match none of the patterns above (green bulk output verified free of `fail`/`Panic`: `BULK Echo:`, `BULK TooLarge rejected`, `BULK Budget rejected`, `BULK Refund ok`, `BULK Verified: 73728`, `BULK Done` — and the connect-retry loop prints nothing). Do NOT assert console silence or line exclusivity.
**Compilability:** test-only crate; nothing else consumes it.
**Gate:** `timeout 1200s cargo xtask test` @root (proven form). Expected roster: host suites unchanged (kernel **250**, shell 5, usrlib 12, system 7, tests-integration lib 18, dummy/test_suite/xtask 0) and **QEMU 9**: app_lifecycle 2 + boot_and_shell 4 + ipc_bulk 1 + ipc_random 1 + ipc_ping 1.
**Commit:** `Add QEMU integration test for ping data roundtrip`

---

## 4. Risks and edge cases (coder must handle / verifier must know)

1. **Repeated `ping` invocations.** Sequential by construction (shell blocks in `wait_future`). Clean exit per BR3 — evidence recorded there. After a disconnect, the server's stale `previous_pong` token answers `NotFound` and is discarded (BR5). No per-invocation state accumulates: connections close (mailboxes + GC), data objects are writer-released or GC-reclaimed, the client task slot is removed by `handle_termination`.
2. **Client hangs at `wait_for_message` on protocol divergence.** If the server ever fails to reply (`receive_data`/`ipc_data_write` error), the client blocks forever and the test times out at SWEEP_TIMEOUT printing the tail — acceptable, unreachable on green; do not add client-side timeouts (protocol surface is locked).
3. **Keystroke echo interleaving.** `send_line("ping")` echoes through the shell prompt line; all assertions use full prefixes (`ping sent: `, `PONG `) — echo cannot fabricate them; the redrawn `rose>` line is ignored.
4. **Extra boot line `[IPC Server] Ping`.** All existing session tests use `contains`-based needles; checked every needle in `boot_and_shell.rs`, `app_lifecycle.rs`, `ipc_random.rs`, `ipc_bulk.rs` — no substring collision in either direction. One extra mostly-blocked task (notification-driven accept wait) does not perturb repaint/idle watchdogs.
5. **Bulk exchange inside the `ipc_ping` window.** The 40 s `STARTUP_DELAY_MS` (`bulk_client/src/main.rs:34`) means the boot bulk exchange runs concurrently with, or right after, the ping sweep in every session. The ping assertion completes well before t+40 s under normal speed, and green bulk output is pattern- and forbidden-string-safe (§Step 4). Never assume the window is ping-only.
6. **Bare forbidden strings over a spawn-marked window.** `"fail"`/`"Panic"` are checked across the whole session text (decision 6). Static sweep: no green-path print of either substring exists in `kernel/`, `arch/`, `usrlib/`, `apps/` (the only kernel hit is a test fn name). Residual: the external `bootloader` crate's banner is not under this sweep — if a future toolchain made it print `fail`, this gate would false-fire; escalate rather than silently narrowing the check.
7. **65536 boundary.** Accepted by the kernel (strict `>`); unit-proven. The 64 KiB round also exercises multi-chunk `receive_data` reads (4096-byte chunks, `usrlib/src/ipc.rs:65`).
8. **Budgets.** Per-task (§BR5/§0): ping never approaches 256 KiB; bulk probe bursts use different tasks; arena is 256 objects system-wide — ping uses ≤ 4 live objects at any time.
9. **Message fit.** Max text 22 bytes + 4-byte token ≤ 60 — by construction; no truncation path needed.
10. **CI blindness.** `ci.yml` never runs `tests-integration`; `ipc_ping` guards local `cargo xtask test` only — same protection level as `ipc_bulk` today (documented Context-Map correction 3).
11. **Host `cargo test --workspace`** builds the shell lib for x86_64-linux where the x86_64 cfg is true — requires `target/rosx-user/release/ping_client` exactly like snake/tetris already do; `xtask test` builds apps first, so the ordering is inherited, not new.

---

## 5. Acceptance criteria (exact commands, all from the proven set)

| # | Command | CWD | Expected |
|---|---|---|---|
| 1 | `timeout 900s cargo test -p kernel -- --test-threads=1` | repo root | `250 passed; 0 failed` (kernel untouched by this work) |
| 2 | `timeout 1200s cargo xtask test` | repo root | all suites green; **QEMU integration roster = 9 tests** incl. `ipc_ping::shell_ping_capability_data_roundtrip ... ok`; host totals unchanged (292) + 9 = 301 |
| 3 | `timeout 900s cargo xtask build` | repo root | reaches `==> create x86_64 disk image`; `^error\|error\[` grep = 0 |
| 4 | `timeout 900s cargo build -p rosx-x86` | `arch/x86_32/` | exit 0; neither ping app compiled (not a dependency); shell warnings unchanged at 3 |
| 5 | `timeout 900s cargo xtask apps` | repo root | 9 ELFs in `target/rosx-user/release/`, incl. `ping_server`, `ping_client` |

**Warning gate — Δ-new = 0 against the recorded branch fingerprint** (`.opencode/artifacts/ipc-bulk-verify-final.md` verdict table), arbitrated **location-first** (normalized `warning text|file` pairs): per-target multiset `{kernel lib 60, kernel lib-test 83 (49 dup — dup-suppressed profile; the 86/52 original is historical, reductions are attributed, never violations), shell lib 3 (shell.rs:132/152/166), rosx bin 26, rosx manifest 1, rosx-x86 bin 1 (main.rs:66), tests-integration lib 1}`. No site is deleted by this plan, so totals must hold identical; decreases are tolerated only with attribution via baseline multiset diff, never treated as violations; totals corroborate, never convict. The counting filter must match **both** bare `warning:` and code-bearing `warning[EXXXX]:` summary lines. The new artifacts (2 app crates, `ipc_ping` test binary) must contribute zero unique warnings. A diagnostic located only in unchanged files is baseline.

**Comment law gate (scripted, portable POSIX grep):**
`grep -nE '(^|[[:space:]])//[^!]' apps/ping_server/src/main.rs apps/ping_client/src/main.rs tests-integration/tests/ipc_ping.rs` → no output; `git diff` shows zero added lines containing `//` or `/*` in `apps/shell/src/shell.rs`, `apps/shell/build.rs`, `arch/x86_64/src/main.rs`, `arch/x86_64/build.rs`, `xtask/src/main.rs`.

**Diff-boundary gate:** `git diff --name-only` against the step-0 baseline must not contain any path under `kernel/`, `system/`, `usrlib/`, `apps/bulk_echo/`, `apps/bulk_client/`, nor `tests-integration/tests/ipc_bulk.rs`.

*(Optional developer shortcut, NOT binding — marked `unverified` because this exact invocation form has no green transcript: `cargo test -p tests-integration --test ipc_ping -- --test-threads=1`. Acceptance is gate 2, which rebuilds everything.)*
