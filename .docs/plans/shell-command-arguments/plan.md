# Plan — Forward Shell Command-Line Arguments to ELF Apps

## Objective

A line typed as `pi alpha beta` at the `rose>` prompt launches `hello_elf`
so that `usrlib::args()` returns `["pi", "alpha", "beta"]` (argc 3).
Today `apps/shell/src/shell.rs` parses those arguments (`Command::parse`)
and drops them at dispatch (`BTreeMap<String, fn()>`).

Base: branch `shell-command-arguments`, forked from `origin/main@5b36efd`
("Forward kernel-provided argc/argv to user-space ELF apps"). The
kernel→app side (ElfTaskContext, C-ABI entry, usrlib `args()/argc()/argv()`)
already exists; this plan wires **shell → wire → kernel**.

## Architecture

Four layers, one concern each:

```
apps/shell          parse (done) → pre-check limits → spawn with argv
usrlib              encode argv → LoadElf arg2/arg3 (new load_with_args)
kernel/syscall      consume arg1 Box FIRST, validate blob, copy, schedule
kernel/task         ElfTaskContext from raw blob → argc/argv to entry (done)
system (treaty)     MAX_ARGUMENT_COUNT=16, MAX_ARGUMENT_BYTES=512, 2 ErrorCode variants
```

### Wire contract (LoadElf = 10, number unchanged)

- `arg1` — unchanged: leaked `Box<&[u8]>` fat pointer to the ELF image.
- `arg2` — pointer to the arguments blob, `0` when absent.
- `arg3` — blob length in bytes, `0` when absent.
- Exactly `(arg2, arg3) == (0, 0)` ⇒ kernel default argv `["ELF"]`
  (preserves bare `Syscall::load` behavior and boot-site semantics).
- Any other pair must be a **canonical blob**: each argument's UTF-8 bytes
  followed by one `0x00` (so the blob ends with `0x00`, which makes the
  existing `install_argument_pointers()` `unwrap()` provably safe).
  Canonicality rules, enforced kernel-side once:
  - `len <= MAX_ARGUMENT_BYTES` (512)
  - NUL count `<= MAX_ARGUMENT_COUNT` (16)
  - non-empty, ends with `0x00`, does not start with `0x00`, no `0x00 0x00`
    run (no empty arguments)

### Failure conventions (kept separate, per review R4)

- `u64::MAX` sentinel — remains ONLY for schedule exhaustion (→ `None`).
- `ErrorCode::ElfArgumentLimitExceeded = 0x0201` — blob longer than
  `MAX_ARGUMENT_BYTES` or more than `MAX_ARGUMENT_COUNT` arguments.
- `ErrorCode::ElfArgumentBlobMalformed = 0x0202` — non-canonical blob or a
  half-set `(ptr, len)` pair.
Both error tokens decode to `None` in `UserFutureHandle::from_token`; the
shell pre-checks the same treaty constants and prints a message, so a
user only ever sees the friendly path (UX, not invariant duplication).

### Ownership of concerns

- `system::args` holds limits only (contract, not mechanism). The
  blob encoding is boundary translation and lives in `usrlib`; the kernel
  never decodes to strings, it copies the blob into the task context and
  scans NULs to build pointers (mechanism).
- The kernel validates once; the shell pre-check exists purely for UX.

## Task graph

```
T1 treaty ──► T2 kernel ──► T3 usrlib ──► T4 shell (+ test expectation) ──► T6 verify
                                             ▲
                    T5 harness (spc fix + new QEMU test) ──────────────────┘
```
Each task leaves the tree green and gets its own commit (imperative
message, no prefix). T2 adds pub items unused until T3/T4 — acceptable
(no dead-code warnings on `pub` API); T4 is the first observable change.

## File manifest

Owned (modified by this effort):
- `system/src/args.rs` (new), `system/src/lib.rs`, `system/src/error.rs`
- `kernel/src/task.rs`, `kernel/src/syscall.rs`
- `usrlib/src/syscall.rs`
- `apps/shell/src/shell.rs`
- `tests-integration/src/sendkey.rs`, `tests-integration/tests/elf_arguments.rs`,
  `tests-integration/tests/shell_command_arguments.rs` (new)

Shared, read-only for this effort:
- `kernel/src/elf/*`, `kernel/src/cpu.rs`
- `usrlib/src/runtime.rs`, `usrlib/src/arch/*`, `usrlib/src/handle/*`
- `apps/shell/src/command.rs` (parser deliberately untouched)
- `arch/x86_64/src/main.rs` (boot sites keep the `["ELF"]` placeholder —
  binding decision D2), all apps except test expectation changes

## Verification

Per task: the commands listed in each `task-N.md`. Final gate (T6):
1. `cargo xtask test --skip-integration` — system registry, kernel task/blob
   validation, usrlib encode, shell parser tests all green.
2. `cargo xtask build` — apps + kernel + disk image build clean.
3. `cargo test -p tests-integration -- --test-threads=1` — updated
   `elf_arguments`, new `shell_command_arguments`, plus existing suites
   (`boot_and_shell` regression for the untouched built-in paths).
4. `cargo build -p rosx-x86` from `arch/x86_32/` — shell still compiles
   for x86_32 (4-arg `raw_syscall` shim already exists there).
5. Manual smoke: `cargo run`, type `pi alpha beta` at `rose>`, expect
   `args(3): ["pi", "alpha", "beta"]` before the Pi digits start.
