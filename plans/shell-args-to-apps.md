# Shell Command-Line Arguments to Apps

> **Date:** 2026-10-18
> **Task:** Deliver command-line arguments from the shell to ELF apps. The shell parses the line, the `LoadElf` syscall carries the argument blob, and `usrlib::runtime` receives it at the app entry and forwards it to the app's `main` under a new signature. All ELF apps are adapted to the new `main` signature. No app consumes arguments semantically in this iteration (task decision: "none for now, we just need the support / feature").

## Resolved decisions (from the requester, binding)

1. `usrlib::runtime` receives the arguments and forwards them to the app entrypoint `main()`; all apps adopt the new `main()` signature.
2. The program name **is** delivered as the first argument (argv[0]).
3. No demo consumer this iteration; behavior of every app is unchanged.
4. Shell keeps whitespace-only splitting; no quote parsing.

## Validated baseline (session evidence, not self-attestation)

- Pinned toolchain, verbatim from `rust-toolchain.toml`: `channel = "nightly-2026-10-03"`.
- Green verbatim this session (the only command with a proven-green transcript):
  `cargo build --release -p hello_elf --target arch/x86_64/rosx-user.json -Zbuild-std=core,alloc,compiler_builtins -Zbuild-std-features=compiler-builtins-mem -Zjson-target-spec`
  Its output carries one baseline warning fingerprint line:
  `warning: the following packages contain code that will be rejected by a future version of Rust: core v0.0.0`
  All warning gates in this plan are **Δ-new = 0** against this baseline (bare `warning:` and code-bearing `warning[EXXXX]:` forms both counted; the line above is baseline-located in the toolchain `core`, not in changed files).
- Every other command embedded below is marked `unverified` and is routed to the orchestrator's baseline pass; an `unverified` gate is not binding until that transcript proves it.

## Structural facts the plan quotes (all read this session)

- `xtask/src/main.rs`: `const USER_APPS: [&str; 11] = ["hello_elf", "random_gen_server", "snake", "tetris", "conway", "bulk_echo", "bulk_client", "ping_server", "ping_client", "vfs_server", "vfs_client"];` — the exact 11 binaries to convert. `apps/dummy` and `apps/test_suite` are **not** in this list (FunctionTask/`exec` world, excluded).
- `usrlib/linker.ld`, verbatim: `ENTRY(main)` and the section pin `*(.text.main)`; `apps/shell/build.rs` and every user build go through `-C link-arg=-Tlinker.ld` from `.cargo/config.toml`. **The entry symbol name must remain `main`.** `linker.ld` must not be edited.
- `usrlib/src/runtime.rs` contains the dead entry shim, verbatim:
  `#[lang = "start"]` / `fn start<T: 'static>(` / `main: fn() -> T,` / `_argc: isize,` / `_argv: *const *const u8,` / `_sigpipe: u8,` / `) -> isize {`.
  Disassembly of the baseline `hello_elf` proves the generated `main` shim calls the user main with no argc/argv plumbing; classic register argv cannot reach the current `fn main()`.
- `kernel/src/task.rs:175`: `pub fn new_elf_task(elf: &'static [u8]) -> SharedTask {` and `kernel/src/task.rs:229`: `pub(crate) extern "C" fn elf_task_wrapper(elf: usize) {` whose body line is `let elf_bytes: &[u8] = unsafe { *Box::from_raw(elf as *mut &[u8]) };`, then `let task_entry_point: fn() = unsafe { core::mem::transmute(image.entry) };` and `task_entry_point();`.
- `kernel/src/syscall.rs:79-86` (LoadElf arm), key line verbatim: `match kernel().schedule(new_elf_task(elf_bytes)).ok() {`.
- `usrlib/src/syscall.rs:19`: `pub fn load(elf: &'static [u8]) -> Option<UserFutureHandle> {`.
- `apps/shell/src/shell.rs`: table `static ref COMMANDS: BTreeMap<String, fn()> = {`; entries of form `(String::from("ls"), ls as fn()),`; cfg inserts of form `commands.insert(String::from("ping"), ping as fn());`; dispatch line `command();`; six launcher call sites `Syscall::load(<NAME>_ELF)` (`shell.rs` lines 122, 128, 134, 140, 147, 154) — the **only** `Syscall::load` callers in the repo (grep-verified).
- `arch/x86_64/src/main.rs:74-83`: five boot spawns, e.g. verbatim `let _ = kernel.schedule(new_elf_task(RANDOM_GEN_SERVER_ELF));` for `RANDOM_GEN_SERVER_ELF`, `BULK_ECHO_ELF`, `BULK_CLIENT_ELF`, `PING_SERVER_ELF`, `VFS_SERVER_ELF`. Line 84: `let _ = kernel.schedule(FunctionTask::new("Shell", shell::shell::main));` — the shell is a kernel-linked lib, **not** an ELF app; its `main` signature is untouched.
- `arch/x86_32/src/main.rs` schedules **zero** ELF tasks (only the FunctionTask shell). x86_32 consumes this feature only by building the same converted apps for `rosx-i686-user` and by the shared kernel changes.
- `arch/x86_64/Cargo.toml:8`, `arch/x86_32/Cargo.toml:8`, `kernel/Cargo.toml:9`, `apps/shell/Cargo.toml:11` all already declare `system` as a dependency. `system/src/lib.rs` already has `extern crate alloc;`.
- Both user arch wrappers take four usize args: `usrlib/src/arch/x86_64.rs` and `usrlib/src/arch/x86_32.rs` both define `pub fn raw_syscall(num: usize, arg1: usize, arg2: usize, arg3: usize) -> usize`.
- `usrlib/src/lib.rs` line 9: `#[cfg(all(feature = "runtime", not(test)))]` gates `mod runtime;`.
- ELF apps' host-test exposure: every `[[bin]]` in the 11 apps sets `test = false` (e.g. `apps/hello_elf/Cargo.toml`), so `cargo test --workspace` never builds them for the host; conversions touch only the `target_os = "none"` world.
- `tests-integration/src/qemu.rs` exposes a scripted QEMU harness (`spawn`, `expect_output`, `send_text`, `send_line`, `mark`, `expect_output_since`, proven by usage in `tests-integration/tests/boot_and_shell.rs`).

## Binding resolutions (contracts)

**BR1 — Args wire blob (`system::args`).** A blob is a sequence of UTF-8 entries concatenated with a single `0x00` byte between consecutive entries; no leading or trailing separator; empty list ⇔ empty blob. `decode` splits on `0x00` and applies `String::from_utf8_lossy`; empty blob ⇒ empty `Vec`; malformed UTF-8 yields replacement characters, never an error; a `0x00` cannot originate from the shell's keyboard stream, but if one were smuggled in, it decodes as an empty entry. **Contract: entry 0 is the program name (argv[0]), placed by the spawner** (shell launcher or kernel boot site). The kernel core treats the blob as opaque transport bytes and never parses it — consistent with "the kernel never interprets message payloads".

**BR2 — Entry call convention.** `elf_task_wrapper` calls the loaded image entry as `extern "C" fn(args_ptr: usize, args_len: usize) -> isize`. The blob bytes are owned by the wrapper's local scope and live across the whole call (the wrapper does not return until the app finishes). When `args_len == 0` the pointer value is unspecified; the callee must not dereference. This works on both arches with no assembly: x86_64 SysV passes the two usizes in registers; x86_32 cdecl passes them on the stack; in both cases the *caller* is our `extern "C"` transmute in `elf_task_wrapper`, so stack cleanup is caller-side and old, argument-less entry shims (pre-conversion binaries, if any image mixes generations) remain safe to call.

**BR3 — Entry symbol stays `main`; alias rename inside the macro.** Because `ENTRY(main)` + `*(.text.main)` are locked facts and rustc rejects both a parameters-taking `main` and two items named `main`, the app-facing surface is: crate-level `#![no_main]` plus a macro invocation that *wraps* the literal text `fn main(<arg>: Vec<String>) { … }`. The macro emits (a) the user's body under a crate-internal alias name, and (b) the exported `#[unsafe(no_mangle)] extern "C" fn main(args_ptr: usize, args_len: usize) -> isize` delegating to `$crate::runtime::start(<alias>, args_ptr, args_len)`. The generated `main` signature's `Vec` must be spelled `::alloc::vec::Vec` so resolution does not depend on app-file imports (all 11 apps already declare `extern crate alloc;`; the conversion step must verify this per file). `linker.ld`, `.cargo/config.toml`, and `apps/shell/build.rs` are not edited.

**BR4 — `LoadElf` extension, no new syscall number.** `SyscallNum::LoadElf = 10` keeps its number. `arg1` unchanged (leaked `Box<&[u8]>` fat pointer). `arg2` = blob pointer, `arg3` = blob length; `arg2 == 0` **or** `arg3 == 0` ⇒ task gets an empty blob. The kernel copies the bytes into kernel-owned storage synchronously during the syscall, so the caller's buffer may die immediately after return (same lifetime contract as `Print` and `IpcDataWrite`).

**BR5 — argv[0] provisioning.** The five `arch/x86_64/src/main.rs` boot spawns encode exactly one entry each: `RANDOM_GEN_SERVER_ELF` → `"random_gen_server"`, `BULK_ECHO_ELF` → `"bulk_echo"`, `BULK_CLIENT_ELF` → `"bulk_client"`, `PING_SERVER_ELF` → `"ping_server"`, `VFS_SERVER_ELF` → `"vfs_server"` (the binary file names). The shell prepends `cmd.name` before `cmd.args`.

**BR6 — usrlib API surface.**
- `pub fn load_with_args(elf: &'static [u8], args: &[String]) -> Option<UserFutureHandle>` on `Syscall` — encodes via `system::args::encode`, passes blob ptr/len as `arg2`/`arg3`; empty `args` ⇒ passes `0, 0`.
- `pub fn load(elf: &'static [u8]) -> Option<UserFutureHandle>` retained, delegating with an empty list (a convenience for spawners with no identity to hand).
- `pub fn start(entry: fn(alloc::vec::Vec<String>), args_ptr: usize, args_len: usize) -> isize` in `usrlib::runtime` — decodes the blob per BR1/BR2, calls `entry`, returns `0`. `mod runtime` becomes `pub mod runtime` under its existing cfg gates; the `macro_export`ed macro references it via `$crate::runtime::start`.
- `#[macro_export] macro_rules! app_main` matching the exact form `fn main($args:ident : Vec < String >) $body:block` (matcher tokens; the generated signature uses `::alloc::vec::Vec<String>`). Invocation form in apps: `usrlib::app_main! { fn main(_args: Vec<String>) { … } }`.
- The `#[lang = "start"]` fn in `runtime.rs` is removed **in the same step** as the last app conversion (Step 6), because intermediate worlds still contain non-`no_main` apps whose rustc-emitted entry references the `start` lang item. Keep `#[panic_handler]` and `#[global_allocator]`. Keep `#![cfg_attr(..., allow(internal_features))]` (needed by `core::panic::PanicInfo`); drop `#![cfg_attr(..., feature(lang_items))]` only if the coder's compile confirms nothing else needs it.

**BR7 — Untouched by design.** The `Exec` path (`task_wrapper`, `fn()`), `apps/dummy`, `apps/test_suite` (both reached via `FunctionTask`/`Syscall::exec`), the shell's own `main` (kernel-linked lib), `usrlib/linker.ld`, `.cargo/config.toml`, `xtask` `USER_APPS` list, `Command::parse` semantics (whitespace split, existing tests stay green verbatim).

## Steps

### Step 1 — `system`: args wire codec

**Files:** `system/src/args.rs` (new), `system/src/lib.rs`.
**Add:** `pub mod args;` in `lib.rs`. In `args.rs`: `pub fn encode(args: &[impl AsRef<str>]) -> Vec<u8>` and `pub fn decode(bytes: &[u8]) -> Vec<String>` implementing BR1 exactly (encode: join with single `0x00`; decode: `split(0x00)` + `from_utf8_lossy`; empty ⇒ empty both ways). State the argv[0]-is-entry-0 contract in the fn names/structure and test names, not comments (repo rule: no comments).
**Test (in-file `#[cfg(test)]`):** roundtrip multi-entry; empty list ⇔ empty blob; single entry no separator; entry containing spaces; entry with invalid UTF-8 byte decodes lossy without panic; decode of `[]` ⇒ empty.
**Why:** the boundary contract (layout only) lives in the treaty crate; kernel, shell, and usrlib share one format implementation, no duplication.
**Verification (`unverified`):** `cargo test -p system -- --test-threads=1`. Whole-workspace compilability is unaffected (additive).

### Step 2 — `usrlib`: runtime entry shim + app macro

**Files:** `usrlib/src/runtime.rs`, `usrlib/src/lib.rs`.
**Modify:** `mod runtime;` → `pub mod runtime;` (same `cfg` attributes).
**Add:** `pub fn start(entry: fn(alloc::vec::Vec<String>), args_ptr: usize, args_len: usize) -> isize` per BR6 — construct the slice only when `args_len != 0` (BR2 pointer rule), decode via `system::args::decode`, call `entry`, return `0`. The `unsafe` block is only the `from_raw_parts`; document-safety by structure (minimal block), per repo `unsafe` policy.
**Add:** `#[macro_export] macro_rules! app_main` per BR3/BR6. Contract of the expansion, in prose: it defines the user body under one crate-internal name and one exported symbol `main` with signature `extern "C" fn(usize, usize) -> isize` carrying `#[unsafe(no_mangle)]`, whose body is a single call to `$crate::runtime::start` with the alias's fn item and the two received values. No other identifiers are introduced into the app namespace except the internal alias.
**Do not remove** `#[lang = "start"]` yet (BR6 rationale).
**Why:** purely additive; both consumer worlds (already-converted-none apps, shell, kernel) build unchanged.
**Verification (`unverified`):** `cargo xtask apps`. Δ-new warning gate against the baseline fingerprint.

### Step 3 — `usrlib`: args-carrying `load`

**Files:** `usrlib/src/syscall.rs`.
**Add:** `pub fn load_with_args(elf: &'static [u8], args: &[String]) -> Option<UserFutureHandle>` per BR4/BR6: `system::args::encode(args)` into a local `Vec<u8>`; keep the existing `Box::into_raw(Box::new(elf))` pattern for `arg1`; pass `blob.as_ptr() as usize, blob.len()` as `arg2`/`arg3` when non-empty, else `0, 0`; decode the return exactly as `load` does today (`UserFutureHandle::from_token`).
**Modify:** `load` body → delegate: `Self::load_with_args(elf, &[])`.
**Why:** additive-first keeps the shell (still calling `load`) buildable at this step; kernel copy semantics (BR4) make the local blob safe.
**Verification (`unverified`):** `cargo xtask build`.

### Step 4 — kernel: carry the blob to the entry point

**Files:** `kernel/src/task.rs`, `kernel/src/syscall.rs`, `arch/x86_64/src/main.rs`.
**Modify `task.rs`:**
- `pub fn new_elf_task(elf: &'static [u8], args: Vec<u8>) -> SharedTask`: `entry_param` becomes `Box::into_raw(Box::new((elf, args))) as usize` (a boxed fat-pointer/`Vec` pair); `Task::new("ELF", elf_task_wrapper as usize, entry_param)` unchanged otherwise. `Task` struct gets no new field — the blob rides the entry parameter and is owned by the wrapper frame (BR2); do not add a `Task::args` field (out of scope, avoids `Task::new` churn across every factory).
- `elf_task_wrapper(entry_param: usize)`: unbox the pair; `load_elf(elf_bytes, kernel().elf_arch).unwrap()` and `kernel().execution_state.preemption_enabled = true;` unchanged; transmute `image.entry` to `extern "C" fn(usize, usize) -> isize` and call it with `(args.as_ptr() as usize, args.len())`; keep `drop(image);` then `kernel().terminate_and_yield();`. The `args` local must stay in scope across the call (drop naturally at scope end).
**Modify `syscall.rs` LoadElf arm:** after unboxing `elf_bytes` (verbatim line kept), build `let args: Vec<u8> = if arg2 != 0 && arg3 != 0 { copy of `core::slice::from_raw_parts(arg2 as *const u8, arg3)` } else { Vec::new() };` (BR4: synchronous kernel-owned copy), then change the schedule line verbatim target from `match kernel().schedule(new_elf_task(elf_bytes)).ok() {` to pass `args` as the second argument.
**Modify `arch/x86_64/src/main.rs`:** the five lines of the quoted form `let _ = kernel.schedule(new_elf_task(RANDOM_GEN_SERVER_ELF));` each gain the BR5 single-entry blob via `system::args::encode`; add the needed `use` of the codec (the crate-level `system` dependency already exists, quote above — no Cargo edit).
**Why:** this is the kernel world as one compilable unit: signature + wrapper + both consumer families (syscall handler, boot sites) land together; no later step is named.
**Blast radius (proves on every process kind that executes it):** kernel unit tests never construct `new_elf_task` (grep-verified: only `Task::new` appears in `kernel/src/task.rs` tests), so `cargo test -p kernel -- --test-threads=1` proves compilation of untouched unit worlds; the ELF-entry convention itself executes **only** in the QEMU integration processes — all five suites boot the image that runs the five boot ELFs plus the shell, and the `ipc_*` suites transitively reach `new_elf_task` through the kernel boot spawns and `Syscall::load`. Old-generation app binaries embedded in this step's image are ABI-safe under BR2's caller-cleans argument passing and simply ignore the two args.
**Verification (`unverified`):** `cargo xtask build` then `cargo xtask test` (full, includes tests-integration, runs with `-- --test-threads=1` per `xtask/src/main.rs`).

### Step 5 — shell: stop discarding `Command.args`

**Files:** `apps/shell/src/shell.rs`.
**Modify:**
- Table type `BTreeMap<String, fn()>` → `BTreeMap<String, fn(&Command)>` (both the `lazy_static!` declaration line and every `as fn()` cast — entries and the two cfg inserts — become `as fn(&Command)`).
- Dispatch line `command();` → `command(&cmd);`.
- Every builtin signature takes the command: `fn ls(_cmd: &Command)`, `fn clear(_cmd: &Command)`, `fn rose(_cmd: &Command)`, `fn sleep(_cmd: &Command)`, `fn random(_cmd: &Command)`, `fn tests(_cmd: &Command)` — builtins ignore args this iteration (decision 3).
- One private helper `fn spawn(elf: &'static [u8], cmd: &Command)`: build `let mut argv = vec![cmd.name.clone()]; argv.extend_from_slice(&cmd.args);` (BR5) then `if let Some(fh) = Syscall::load_with_args(elf, &argv) { Syscall::wait_future(fh); }`; `pi`, `snake`, `tetris`, `conway` and the cfg'd `ping`, `vfs` become one-line delegations to `spawn` with their existing `*_ELF` statics.
**Grep gate:** `grep -En 'Syscall::load\(' apps/shell/src/shell.rs` returns no lines (all six sites became `load_with_args` via `spawn`; the pattern must not match the helper's call — it matches only the exact `load(` form).
**Why:** the parsed-but-discarded `args` become live end-to-end; table shape change is the smallest edit that threads `&Command` through.
**Blast radius:** `apps/shell/src/command.rs` tests are parse-only and unchanged; the shell's `fn(&Command)` world is host-compiled by the workspace default-members, so the unit gate covers it, and `boot_and_shell.rs` covers it in QEMU.
**Verification (`unverified`):** `cargo xtask test --skip-integration` and `cargo xtask build`.

### Step 6 — convert the 11 ELF apps; retire the lang-item shim

**Files (all mechanical):** `apps/<name>/src/main.rs` for each of the 11 `USER_APPS` names (structural enumeration quoted above).
**Modify each app:**
- Add `#![no_main]` beside the existing `#![no_std]`.
- Verify the file contains `extern crate alloc;` (add if — expectedly — absent; BR3 path requirement).
- Wrap the entry with `usrlib::app_main! { fn main(_args: Vec<String>) { … } }` — parameter named `_args` (no consumer yet; decision 3), body byte-identical otherwise.
**File set closed under `main` renames:** `apps/dummy/src/app.rs` and `apps/test_suite/src/app.rs` are `pub fn main()` on the FunctionTask/`exec` path (BR7) — **not** converted; grep evidence: they appear in no `[[bin]]`/`USER_APPS` structure.
**Also in this step:** remove the `#[lang = "start"] fn start<T: 'static>(…)` block from `usrlib/src/runtime.rs` (now zero references: every ELF world consumer is `#![no_main]`, every host world consumer never had one), and drop `feature(lang_items)` from the two `usrlib/src/lib.rs` `cfg_attr` lines iff the compile is clean without it; keep `allow(internal_features)`.
**Why:** all-11 conversion plus shim removal is one commit because the lang item and the last `main`-entry app are mutually-locking (any smaller cut leaves a world that fails to link).
**Blast radius:** every QEMU suite (`boot_and_shell`, `ipc_random`, `ipc_ping`, `ipc_vfs`, `ipc_bulk`) now runs new-generation binaries through the BR2 convention — servers spawned by kernel boot (RANDOM, BULK_ECHO, BULK_CLIENT, PING_SERVER, VFS_SERVER) and clients spawned by the shell; `apps/shell/build.rs` existence checks keep validating the same 6 embedded ELF names (unchanged names, rebuilt contents). Host `cargo test --workspace` never builds these bins (`test = false`, quote above).
**Grep gates:** `grep -rLE '#!\[no_main\]' apps/*/src/main.rs` returns empty; `grep -rEn 'lang ?= ?"start"' usrlib/src/` returns no lines.
**Verification (`unverified`):** `cargo xtask apps`, `cargo xtask build`, then `cargo xtask test` (full).

### Step 7 — documentation sweep

**Files:** `AGENTS.md`.
The sweep found exactly one invalidated line: `AGENTS.md:149` verbatim `- \`elf/\` — loads standalone user-space ELF binaries into tasks (\`new_elf_task\`).` → amend to state that `new_elf_task` also carries the spawner's argument blob to the entry. Keywords `LoadElf`, `Syscall::load`, `elf_task_wrapper`, `BTreeMap`, `fn main` have no other hits in `AGENTS.md`, `CLAUDE.md`, `GEMINI.md`, `README.md`, `docs/todo.md` (grep evidence above; `CLAUDE.md`/`GEMINI.md` differ from `AGENTS.md` but contain none of these). No new app or workspace member appears anywhere, so app inventories and the `apps/` structure listing stay correct.
**Verification:** none (markdown-only step).

### Step 8 — scripted end-to-end smoke (new integration test)

**Files:** `tests-integration/tests/app_args_plumbing.rs` (new).
**Contract:** a test that spawns `QemuSession`, expects the `rose>` prompt, `mark()`, `send_line("pi")`, and `expect_output_since(mark, "Pi:", …)` — proving a converted, args-carrying binary loads and runs to first output under the new entry convention; then a second test segment `send_line("foo 123")` expecting `Unknown command: foo` — proving extra args never derail command dispatch. Mirror the style of `boot_and_shell.rs` (same helper API, durations from that file).
**Why:** the QEMU harness is scripted tooling, so the runtime gate for "the plumbing runs" is machine-countable, not manual.
**Verification (`unverified`):** `cargo xtask build` then `cargo test -p tests-integration -- --test-threads=1`.

## Risks and edge cases the coder must handle

1. **Entry symbol drift:** any rename of the macro-emitted `main` breaks `ENTRY(main)`/`*(.text.main)`. The plan forbids touching `linker.ld`; the grep gate `grep -E 'ENTRY\(main\)' usrlib/linker.ld` must remain true unmodified.
2. **`args_len == 0` pointer rule (BR2):** neither side may form a slice from `Vec::new`'s dangling pointer; kernel forms the copy only when `arg2 != 0 && arg3 == 0` is false and `arg2 != 0 && arg3 != 0` (i.e., both nonzero); usrlib `runtime::start` forms the slice only when `args_len != 0`.
3. **Mixed-generation ABI window (Step 4 → Step 5):** old argument-less entry shims are called with two extra args; safe under both SysV and cdecl because the caller (our transmute) owns argument space. Never switch the transmute to `extern "Rust"`.
4. **Old binaries in image cache:** `apps/shell/build.rs` fails the build if `target/rosx-user/release/*` ELFs are missing; a stale pre-conversion ELF silently mixes generations after Step 6 only if `cargo xtask build` (which rebuilds apps first) is skipped — the gate sequence `cargo xtask apps` before `cargo xtask build` (or plain `cargo xtask build`) closes this.
5. **x86_32 parity:** same blob convention through `int 0x80`'s 4-register wrapper (quote above); `extern "C"` two-usize call is cdecl-correct there. No x86_32 boot spawn sites exist. Build gate below; marked `unverified` (no transcript).
6. **NUL smuggling / non-UTF-8:** unreachable from the shell keyboard path; codec behavior is defined (BR1), never panics.
7. **Macro matcher strictness:** `usrlib::app_main!` accepts only `fn main(<ident>: Vec<String>) { … }` token shape; apps must not write `pub fn main` or `&[String]` — conversion is mechanical and uniform.
8. **`lang_items` feature removal:** if the Step-6 compile still needs `feature(lang_items)`, keep the gate line and record it here as a retained item rather than forcing the removal.
9. **QEMU slowness:** `pi` generates 20000 digits; the smoke test must only await the `Pi:` prefix, with a duration no shorter than the existing 10s style used after marks in `boot_and_shell.rs`, and never wait for the app to finish.
10. **Manual-only residue:** observing the *contents* of argv at runtime cannot be scripted without an app echoing them, which decision 3 forbids; it stays a manual gate: boot QEMU, run `pi 12` interactively, confirm identical Pi output (app ignores args today) and no fault — evidence of no-crash wiring; argv-content correctness is instead proven by Step-1 codec unit tests plus the ptr/len wiring exercised (crash-detectable) by Step 8.

## Acceptance criteria (exact commands)

1. `cargo xtask test --skip-integration` — green. *(`unverified`)*
2. `cargo xtask build` — green, disk image produced at `target/rosx/release/rosx-x86_64.img`. *(`unverified`)*
3. `cargo test -p tests-integration -- --test-threads=1` (after step 2) — green, including the new `app_args_plumbing.rs`; machine-countable runtime evidence. *(`unverified`)*
4. x86_32 kernel: `cargo build -p rosx-x86` executed from `arch/x86_32/` (its `.cargo/config.toml` supplies the custom target). *(`unverified` — CI has no x86_32 job; recipe source is `AGENTS.md`)*
5. Static gates, each returning the stated result: `grep -En 'Syscall::load\(' apps/shell/src/shell.rs` → empty; `grep -rLE '#!\[no_main\]' apps/hello_elf/src/main.rs apps/random_gen_server/src/main.rs apps/snake/src/main.rs apps/tetris/src/main.rs apps/conway/src/main.rs apps/bulk_echo/src/main.rs apps/bulk_client/src/main.rs apps/ping_server/src/main.rs apps/ping_client/src/main.rs apps/vfs_server/src/main.rs apps/vfs_client/src/main.rs` → empty; `grep -rEn 'lang ?= ?"start"' usrlib/src/` → empty; `grep -E 'ENTRY\(main\)' usrlib/linker.ld` → the verbatim `ENTRY(main)` line, file unmodified vs baseline.
6. Warning gate: Δ-new = 0 against the baseline fingerprint (the toolchain-`core` deprecation `warning:` line recorded above), counting both `warning:` and `warning[EXXXX]:` forms; a decrease at deleted sites is not a violation.
7. Manual residue (justified in risk 10): interactive `pi 12` on QEMU shows unchanged Pi output and a clean shell prompt return.
