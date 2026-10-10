# Plan: ipc_random integration test — shell <-> random_gen_server capability-handle IPC over QEMU

> 2026-10-09 · `ipc-capability-handle` @ `af22a92` · test-only. New `tests-integration/tests/ipc_random.rs` + additive edits to `tests-integration/src/{qemu,output}.rs`. Zero changes to `kernel/ system/ usrlib/ apps/ arch/ xtask/ .github/`, any `Cargo.toml`, README. No comments in any code (AGENTS.md).

## 0. Validated baseline (verified at af22a92)

- `tests-integration` (crate `tests_integration`) is a workspace member (root `Cargo.toml:18`) but not a default-member (`:20-29`); CI runs only `cargo xtask test --skip-integration` (`.github/workflows/ci.yml:31` → `--exclude tests-integration`, `xtask/src/main.rs:80`) → never runs in CI → no `#[ignore]`.
- `src/qemu.rs`: `QemuSession.output: SharedOutput` private (l.22-28). L13 verbatim: `use crate::output::{self, SharedOutput};`. `spawn()` l.31-100: private `TempDir` + `<tempdir>/monitor.sock` (l.32-35) → parallel-safe; `kernel_build::kernel_elf()` (l.33) runs BEFORE QEMU spawn; `-debugcon stdio` (l.40-41) carries all guest output. `expect_output` l.109-111 and `expect_output_since` l.117-119 → `expect_from_mark` l.121-140, which PANICS on timeout. `mark(&self) -> usize` l.113-115 = raw-buffer byte length. `send_line` l.157-160 (50 ms/key). `Drop` l.184-235 quit+reap+rmdir.
- `src/output.rs`: `DIAGNOSTIC_TAIL_CHARS = 2000` (l.6); `wait_for_since(&self, mark: usize, pred: impl Fn(&str) -> bool, timeout: Duration) -> Result<usize, String>` (l.35-60): `let start = mark.min(buffer.len());`, pred over `strip_ansi(&buffer[start..])`, timeout → `Err(2000-char stripped tail)`. `tail(n)` l.70-73 strips the WHOLE buffer (byte marks not re-appliable). Private `snapshot()` l.79-84; `pub fn strip_ansi` l.87-109; reader appends `String::from_utf8_lossy` chunks (l.111-122). Lib unit tests: 18 (qemu 3 + sendkey 5 + output 10).
- `src/kernel_build.rs`: `ROSX_KERNEL_ELF` shortcut l.7-17 — must NOT be set; else OnceLock kernel build per test-binary (l.19-21). `cargo test` has no per-test timeout → kernel build is un-timed; all 60/180 s clocks start only after QEMU spawn.
- `sendkey::token_for` (`src/sendkey.rs:37-65`) maps all of `random` + `'\n' => Some("ret")`. `tests/boot_and_shell.rs`: 4 tests, no `#[ignore]`, 60 s expects, `mark()`+`expect_output_since` — the idiom to mirror.

Guest strings (verbatim). `apps/shell/src/shell.rs`: `static PROMPT: &str = "\x1B[32mrose>\x1B[m ";` (l.29), printed only by `prompt()` (l.82-84), after each command returns (l.69). `fn random()` l.130-153: outer `for i in 1..260` (259 iterations), inner `for i in 1..2` (1): `println!("RANDOM Server: {} {}", ipc_connection.index, ipc_connection.generation);` (:134); `println!("RANDOM Value requested");` (:138); `Ok(value) => println!("RANDOM Value: {}", value),` (:140); `println!("RANDOM Value not received: {}", e)` (:141); `println!("RANDOM Failed to send: {}", result);` (:145); `println!("Connection failed");;` (:151 — stray `;;` pre-existing; do NOT fix). ⇒ exactly 259 of each success line per typed `random`; echo is lowercase `random`, never matches `RANDOM`/`rose>`.
`apps/random_gen_server/src/main.rs`: `println!("[IPC Server] Random");` (:58) once at boot, `[IPC Server] Random - Panic!` (:65) on panic, disposes via `ReadableMailbox::new(msg.buffer_handle)` (:42), silent after boot; auto-started every boot (`arch/x86_64/src/main.rs:74-75`).
256-slot connection arena (`kernel/src/ipc/ipc_manager.rs:45`) ⇒ 259 connect/disconnect cycles wrap generation once (observed at HEAD: `0..=255 @ gen 0`, then `0..=2 @ gen 1` — do not hard-assert). `Handle { pub index: HalfSize, pub generation: HalfSize }` (`collections/src/generational_arena.rs:14-16`) ⇒ numbers ≤ 4294967295, `usize`-parseable on host. `"Buffer not found"` / `"Buffer pool exhausted"` are `IpcBufferError` Display (`system/src/ipc.rs:53-56`), reach output only via :141/:145.

## 1. Binding resolutions (verbatim)

**BR-A — Entire harness diff (additive; `SharedOutput` never exposed — no `push` leak):**
- `QemuSession`: `pub fn wait_until_since(&self, mark: usize, pred: impl Fn(&str) -> bool, timeout: Duration) -> Result<usize, String>` — body: `self.output.wait_for_since(mark, pred, timeout)`; never panics internally; pred receives ANSI-stripped `buffer[mark..]`.
- `QemuSession`: `pub fn content_since(&self, mark: usize) -> String` — body: `strip_ansi(&self.output.snapshot_from(mark))`.
- `SharedOutput`: `pub fn snapshot_from(&self, mark: usize) -> String` — raw `buffer[mark.min(len)..]`, same lock/`into_inner` idiom as `snapshot` (output.rs:79-84). Needed because `tail()` strips the whole buffer; post-mark strip must match `wait_for_since` (l.44-46).
- `src/qemu.rs` L13 becomes: `use crate::output::{self, strip_ansi, SharedOutput};`

**BR-B — Window `[mark, EOF)`**; `mark` = raw byte offset; a mark inside an escape leaks at most the literal tail (`[m`) at window start — cannot create/destroy `RANDOM` prefixes, `rose>`, or failure substrings. Accepted.
**BR-C — Completion gate:** `wait_until_since(mark, |delta| delta.contains("rose>"), SWEEP_TIMEOUT)` — condvar wait, no sleeps. Sound: post-mark the guest emits only echo `random`, newline, `RANDOM …` lines until the final `prompt()` (shell.rs:69).
**BR-D — ONE scan:** `let content = session.content_since(mark);` — every assertion reads that same `String`.
**BR-E — Counts:** exactly `259` `starts_with` matches for each of `"RANDOM Server: "`, `"RANDOM Value requested"`, `"RANDOM Value: "` (trailing space included). Parse `RANDOM Server:` lines into `(index, generation)`; assert SOME `generation > first_observed_generation`; NEVER the literal `0..=255@0 / 0..=2@1` pattern.
**BR-F — Values:** `line["RANDOM Value: ".len()..].trim().parse::<usize>()`; consecutive values pairwise `!=` (NOT monotonic). Parse failure ⇒ panic with the raw line; never filter_map-swallow.
**BR-G — Zero occurrences in window:** `"RANDOM Value not received"`, `"RANDOM Failed to send"`, `"Connection failed"`, `"[IPC Server] Random - Panic!"`, `"Buffer pool exhausted"`, `"Buffer not found"`.
**BR-H — Every failure message embeds:** three counts, first+max generation, values length, last 2000 CHARACTERS of window (`chars()`-tail — reader output is lossy UTF-8, byte slicing can panic).
**BR-I — Target selection (binding):** `--test ipc_random` is the only correct single-target selection (target name = file stem). `cargo test -p tests-integration ipc_random` is a positional name FILTER matching ZERO tests (test fn name doesn't contain `ipc_random`) and exits 0 — vacuous pass; forbidden in all gates.
**BR-J — No `#[ignore]`; no CI/xtask/Cargo.toml/README edits; `ROSX_KERNEL_ELF` unset; no `--test-threads` requirement.**

## 2. Steps (each leaves tree compiling; run the Verify before the next)

### Step A — harness
A1. `tests-integration/src/output.rs` — `impl SharedOutput`, after `snapshot`: add `snapshot_from` (BR-A).
A2. `tests-integration/src/qemu.rs:13` — add `strip_ansi` to the use list (exact line, BR-A).
A3. `tests-integration/src/qemu.rs` — `impl QemuSession`, after `expect_output_since` (ends l.119): add both BR-A methods (`Duration` imported at l.8).
**Verify A** (intermediate gate = pre-existing suite; new test not yet written):
- `cargo check -p tests-integration` → clean, zero warnings
- `cargo test -p tests-integration --lib` → 18 passed; 0 failed

### Step B — `tests-integration/tests/ipc_random.rs` (new file, auto-discovered)
Imports: `use std::time::Duration; use tests_integration::qemu::QemuSession;`
Consts: `READY_TIMEOUT` 60 s; `SWEEP_TIMEOUT` 180 s; `ITERATIONS: usize = 259`; `FAILURE_STRINGS: [&str; 6]` per BR-G.
Private helpers (names free, contracts binding): `count_prefixes(content, prefix)` = `content.lines().filter(|line| line.starts_with(prefix)).count()`; `server_pairs(content) -> Vec<(usize, usize)>` (BR-E + parse-panic guard); `values(content) -> Vec<usize>` (BR-F + guard); `tail_chars`; `window_diagnostics(content) -> String` (BR-H).
One test: `#[test] fn shell_random_capability_ipc_roundtrip()`:
1. `let mut session = QemuSession::spawn();`
2. `session.expect_output("[IPC Server] Random", READY_TIMEOUT);` then 3. `session.expect_output("rose>", READY_TIMEOUT);`
4. `let mark = session.mark();` 5. `session.send_line("random");`
6. BR-C gate; on `Err(tail)`: `panic!("timed out after {SWEEP_TIMEOUT:?} waiting for the random sweep to complete; output tail:\n{tail}")`
7. `let content = session.content_since(mark);` then — on `content` only, every assert embedding `window_diagnostics(&content)`:
   - `assert_eq!(count_prefixes(&content, "RANDOM Server: "), ITERATIONS)` — likewise for `"RANDOM Value requested"`, `"RANDOM Value: "`
   - `pairs.len() == ITERATIONS`; `pairs.iter().any(|(_, g)| *g > pairs[0].1)`
   - `vals.len() == ITERATIONS`; `vals.windows(2).all(|w| w[0] != w[1])`
   - `for f in FAILURE_STRINGS { assert!(!content.contains(f)) }`
8. Exit; Drop quits QEMU + removes TempDir.
**Verify B:** `cargo test -p tests-integration --test ipc_random` → `1 passed; 0 failed; 0 ignored` (first run includes un-timed OnceLock kernel build).

## 3. Risks / edge cases

- TCG timing: sweep ~22-25 s observed → 180 s ≈ 7x margin; 60 s boot gates mirror the contention-proven `boot_and_shell.rs`; all waits condvar, zero sleeps.
- Kernel build inside `spawn()` (qemu.rs:33) precedes all timers; no cargo test timeout; two test binaries serialize on the cargo target-dir lock; no `ROSX_KERNEL_ELF` shortcut (BR-J).
- Counts race-free: all 777 `RANDOM` lines precede completion `rose>` on one ordered debugcon pipe; server silent after boot → any count flake is a real ordering bug; diagnostics prove it.
- Mark/escape split (BR-B): literal `[m` at window start at worst, mark taken while guest idle. Lossy UTF-8 (`output.rs:117`): all slicing/tails via `chars()`; parse guards panic with raw line, never swallow.
- Gen-wrap determinism: fresh arena per boot, one bind, one `random` → `gen > first` robust across boots; literal pattern banned (BR-E). Contamination: per-test QEMU/TempDir/monitor.sock (qemu.rs:32-35); boot suite never types `random`.
- Blast radius: `QemuSession` consumers = `boot_and_shell.rs` (4) + 18 lib tests; change additive; acceptance exercises every process kind (`--lib`, whole suite, `--test`, CI regression).

## 4. Acceptance criteria (verbatim; from workspace root; pinned nightly)

1. `cargo check -p tests-integration` — zero warnings.
2. `cargo test -p tests-integration --lib` — 18 passed; 0 failed.
3. `cargo test -p tests-integration --test ipc_random` — `1 passed; 0 failed; 0 ignored`. Never the positional form (vacuous, BR-I).
4. `cargo test -p tests-integration` — single invocation: 18 lib + 4 boot + 1 ipc_random, 0 failed; record wall time in commit message.
5. `cargo xtask test --skip-integration` — unchanged-green (regression guard for the harness edit; CI's own command, ci.yml:31).
6. Comments check, pass = no output: `grep -nE '(^|[^:])//|/\*' tests-integration/src/qemu.rs tests-integration/src/output.rs tests-integration/tests/ipc_random.rs`
7. Scope: `git status --porcelain` shows only ` M tests-integration/src/qemu.rs`, ` M tests-integration/src/output.rs`, `?? tests-integration/tests/ipc_random.rs` (+ this plan file).
