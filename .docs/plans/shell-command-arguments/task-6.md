# T6 — Full verification

Dependencies: T1–T5. Modifies: nothing (evidence-gathering; fixes a change
reveals belong to go back to their owning task/commit).

## Gate

1. `cargo xtask test --skip-integration`
   Expected green: system registry (15 codes), kernel task/validation
   tests, usrlib encode tests, shell command parser tests.
2. `cargo xtask build` — apps + kernel + disk image build clean.
3. `cargo test -p tests-integration -- --test-threads=1`
   - `elf_arguments` → `args(1): ["pi"]`
   - `shell_command_arguments` → `args(3): ["pi", "alpha", "beta"]`
   - regression: `boot_and_shell`, `ipc_random`, `ipc_ping`, `ipc_vfs`,
     `ipc_bulk`, `app_lifecycle` (built-in commands untouched).
4. `cargo build -p rosx-x86` from `arch/x86_32/` — shell compiles for
   x86_32 (4-arg `raw_syscall` shim already carries arg2/arg3).
5. Manual smoke (report, not automated): `cargo run`; at `rose>` type
   - `pi alpha beta` → `args(3): ["pi", "alpha", "beta"]`
   - `pi` → `args(1): ["pi"]`
   - `snake extra words` → snake starts (args accepted, ignored by app)
   - `clear garbage` → clears quietly (built-in ignores trailing args)

## Report

Raw pass/fail facts per command, with the QEMU test names and durations.
