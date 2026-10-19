# Context Map — Shell Command-Line Argument Forwarding

Base: `shell-command-arguments` branch, forked from `origin/main@5b36efd`
("Forward kernel-provided argc/argv to user-space ELF apps").

## Task

The shell parses `name args...` lines but drops the arguments at dispatch.
Forward them end-to-end so a shell-launched ELF app sees them through the
already-merged argc/argv mechanism.

## Current state at base (origin/main@5b36efd)

### Mechanism (exists)

- `kernel/src/task.rs`
  - `pub struct ElfTaskContext { elf_bytes: &'static [u8], argument_strings: Vec<u8>, argument_pointers: Vec<*const u8> }`
  - `ElfTaskContext::new(elf_bytes, arguments: &[&str])` — joins args with `0x00`
    into `argument_strings`; `install_argument_pointers()` builds a
    NULL-terminated `Vec<*const u8>` into that blob. Unit-tested in-file
    (`elf_task_context_stores_arguments_as_nul_terminated_strings`, etc.).
  - `pub fn new_elf_task(elf: &'static [u8]) -> SharedTask` — hardcodes
    `&["ELF"]` as argv; no per-launch argument path.
  - `extern "C" fn elf_task_wrapper(context: usize)` — transmutes entry to
    `extern "C" fn(i32, *const *const u8) -> i32` and calls
    `task_entry_point(context.argc(), context.argv())`; context outlives the
    call (owned `Box`, dropped after entry returns).
- `usrlib/src/runtime.rs` — `#[lang = "start"]` stores argc/argv into
  `ARGUMENT_COUNT: AtomicI32` / `ARGUMENT_VECTORS: AtomicPtr<u8>`; exposes
  `argc()`, `argv()`, `args() -> Vec<&'static str>`; re-exported in
  `usrlib/src/lib.rs` (`pub use runtime::{args, argc, argv}`).
- Consumer today: `apps/hello_elf` prints `args({}): {:?}` at startup.
- Address space is flat: user pointers are dereferenceable by the kernel
  (precedent: `LoadElf`/`IpcConnect` receive leaked `Box` pointers and
  `Box::from_raw` them synchronously in `kernel/src/syscall.rs`).

### Launch paths (no arguments yet)

- User-space launch: `Syscall::load(elf: &'static [u8]) -> Option<UserFutureHandle>`
  (`usrlib/src/syscall.rs:20`) leaks `Box<&[u8]>` in arg1; `arg2/arg3` unused.
  Handler `SyscallNum::LoadElf` (`kernel/src/syscall.rs:80`) calls
  `new_elf_task(elf_bytes)`; failure returns `u64::MAX as usize` → `None`.
- Boot launch: `arch/x86_64/src/main.rs:75-83` schedules 5 ELF servers via
  `new_elf_task(...)` — all receive placeholder argv `["ELF"]`.
  `arch/x86_32` boots no ELF tasks.
- `Syscall::Exec` (`Exec = 2`) launches kernel-linked fn pointers
  (`test_suite`) — args conceptually inapplicable there.
- Syscall ABI: 3 raw `usize` args + `usize` return
  (`handle_syscall(num, arg1, arg2, arg3)`); `usrlib/src/arch/{x86_64,x86_32}.rs`
  both expose `raw_syscall(num, arg1, arg2, arg3)`.
- Error convention: `system::error::{ERROR_BIT, is_token, ErrorCode}` —
  `#[repr(u32)]` ErrorCode with per-domain codes, `to_reg()/from_code()`,
  registry test `ALL_ERROR_CODES` lists every variant.

### Shell (the gap)

- `apps/shell` is a `no_std` **library linked into the kernel image**
  (`arch/x86_64/src/main.rs:85`, `arch/x86_32/src/main.rs:56`); uses
  `usrlib::syscall::Syscall` wrappers. Not in xtask `USER_APPS`.
- `apps/shell/src/command.rs` — `pub struct Command { name: String, args: Vec<String> }`;
  `Command::parse(line)` whitespace-splits into name + args; 5 unit tests
  already cover argument parsing.
- `apps/shell/src/shell.rs:39` — `COMMANDS: BTreeMap<String, fn()>`; dispatch
  `command()` at `:76-77` **drops `cmd.args`**.
- ELF-backed commands embed binaries via `include_bytes!`
  (`PI_ELF`=hello_elf, SNAKE, TETRIS, CONWAY; x86_64-only: PING, VFS) and
  launch with `Syscall::load(X_ELF)` + `Syscall::wait_future(fh)`.
- Built-ins not ELF-launching: `ls`, `clear`, `rose`, `tests` (via
  `Syscall::exec`), `sleep`, `random` (IPC to boot-scheduled server).
- Error-printing convention: `println!("Unknown command: {}", cmd.name)`.

### Test infrastructure

- Unit: `cargo xtask test`; shell parser tests in `apps/shell/src/command.rs`;
  kernel elf/task tests `cargo test -p kernel -- --test-threads=1`.
- QEMU scripted sessions: `tests-integration/src/qemu.rs` —
  `QemuSession::spawn/mark/expect_output_since/send_line`. Suite example:
  `tests-integration/tests/elf_arguments.rs` types `pi`, expects
  `args(1): ["ELF"]` (this expectation changes once the shell forwards argv[0]).
- Known harness bug: `tests-integration/src/sendkey.rs:49` maps `' ' => "sp"`;
  QEMU `sendkey` token for space is `spc`. Scripted typing of arguments is
  broken until fixed (reference fix: `9eeed7d` on branch `shell-args-to-apps`).

### Conventions

- Treaty constants: plain `pub const NAME: usize` in `system`
  (precedent `system/src/ipc.rs`: `MESSAGE_BYTES`, `MESSAGE_PAYLOAD_BYTES`).
- Treaty holds contract (numbers/layout), not mechanism; kernel is the single
  authority on invariants (validate once, kernel-side).
- No comments in Rust code (assembly exempt); `#[cfg(test)]` in-file modules
  with descriptive test names; commit messages imperative, no prefix.
- Apps build via `cargo xtask apps` / `build`; kernel via `cargo build -p rosx`;
  `cargo build --workspace` unsupported.

## Obsolete references

Branches `shell-args-to-apps` (tip `48eb9e1`) and `elf-argc-argv`
(consumed into main by `5b36efd`) contain related experiments. The
`shell-args-to-apps` branch proves the `LoadElf` arg2/arg3 blob-wire idea and
carries the sendkey `spc` fix, but uses an incompatible app-entry ABI
(opaque blob instead of argc/argv) and is NOT the design basis.
