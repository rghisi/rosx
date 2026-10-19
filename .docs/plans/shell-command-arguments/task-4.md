# T4 — Shell: dispatch commands with their arguments

Dependencies: T3. Modifies: `apps/shell/src/shell.rs`,
`tests-integration/tests/elf_arguments.rs`.

## Objective

Stop dropping `cmd.args` at dispatch. Every ELF-spawning command forwards
`argv = [name] ++ args` through `Syscall::load_with_args`; built-ins keep
their behavior and quietly ignore trailing words.

## Changes (`apps/shell/src/shell.rs`)

1. Dispatch table: `BTreeMap<String, fn(&Command)>` (cast sites become
   `ls as fn(&Command)` etc.); dispatch call becomes `command(&cmd)`.
2. One spawn helper used uniformly by `pi`, `snake`, `tetris`, `conway`,
   and x86_64-only `ping`, `vfs`:
   ```rust
   fn spawn(elf: &'static [u8], cmd: &Command)
   ```
   - Pre-check UX with treaty constants (already a dependency of `shell`):
     - `cmd.args.len() + 1 > system::args::MAX_ARGUMENT_COUNT` →
       `println!("Too many arguments: maximum is {}", MAX_ARGUMENT_COUNT)`, return.
     - encoded size (`cmd.name.len() + sum(cmd.args lens) + argument count`)
       `> system::args::MAX_ARGUMENT_BYTES` →
       `println!("Arguments too large: maximum is {} bytes", MAX_ARGUMENT_BYTES)`, return.
   - Build `Vec<String>` = `cmd.name` clone, then `cmd.args` clones.
   - `if let Some(fh) = Syscall::load_with_args(elf, &argv) { Syscall::wait_future(fh); }`
     (same silent-on-None shape as today; over-limit can't reach the syscall).
3. ELF handlers become `fn pi(cmd: &Command) { spawn(PI_ELF, cmd) }` etc.
   Built-ins (`ls`, `clear`, `rose`, `tests`, `sleep`, `random`) take
   `_cmd: &Command` and ignore it. `apps/shell/src/command.rs` untouched.

## Also in THIS commit (review blocker R1)

`tests-integration/tests/elf_arguments.rs` expectation changes from
`args(1): ["ELF"]` to `args(1): ["pi"]` — the shell now supplies argv[0],
so leaving the old string would make the QEMU suite red after this commit.
The updated test actually runs at T5/T6 (QEMU budget), not in the per-commit
unit gate.

## Acceptance criteria

- `cargo xtask test --skip-integration` green (parser tests unaffected).
- `cargo xtask build` green for x86_64.
- `cargo build -p rosx-x86` from `arch/x86_32/` green (same shell code,
  x86 ELF includes already exist there).
- Manual (optional at this step): `cargo run`, `pi alpha beta` →
  `args(3): ["pi", "alpha", "beta"]`.

Commit: `Forward parsed command arguments from the shell to ELF apps`
