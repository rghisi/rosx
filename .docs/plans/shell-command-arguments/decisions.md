# Decisions — Shell Command Argument Forwarding

## User decisions (interview, binding)

- **D1 — Fresh branch from latest `origin/main`.** Realized as branch
  `shell-command-arguments` forked at `origin/main@5b36efd`, which already
  merged the kernel→app argc/argv mechanism (`ElfTaskContext`, C-ABI entry,
  `usrlib::args()`). The older `shell-args-to-apps` experiment (opaque-blob
  ABI, all apps converted) is NOT the design basis.
- **D2 — argv[0]: shell only.** The shell always prepends the typed command
  name as argv[0] (`pi alpha` → `["pi", "alpha"]`). The five x86_64
  boot sites keep the `["ELF"]` placeholder; `new_elf_task` signature is
  unchanged. `elf_arguments.rs` expectation updates to `args(1): ["pi"]`.
- **D3 — Treaty limits 16 args / 512 bytes.** `system::args` constants;
  kernel enforces once on the LoadElf path returning new ErrorCode variants;
  shell pre-checks the same constants purely for friendly UX (invariant
  authority stays kernel-side).
- **D4 — Tokenization and coverage unchanged scope.** Reuse the existing
  unit-tested `Command::parse` whitespace split (no quoting/escapes). All
  six ELF-spawning commands (`pi`, `snake`, `tetris`, `conway`, `ping`,
  `vfs`) forward arguments uniformly through one spawn helper; built-ins
  (`ls`, `clear`, `rose`, `sleep`, `random`, `tests`) ignore trailing words
  silently.

## Design-review resolutions (binding)

- **R1 — Test expectation travels with the shell commit (T4).** Shipping
  argv[0] forwarding without it would leave the QEMU suite red.
- **R2 — LoadElf consumption order.** Null-check `arg1`, then
  `Box::from_raw` unconditionally as the first act, so every validation
  early-return drops the leaked `Box<&[u8]>` naturally; only then inspect
  `arg2/arg3`. No new early returns between `Box::into_raw(context)` and
  `schedule(..)`.
- **R3 — Trailing NUL is an explicit canonical rule** with a dedicated
  reject test (`b"a\0b"` → malformed); it is what makes
  `install_argument_pointers()`'s `unwrap()` provably safe.
- **R4 — Failure conventions stay disjoint.** `u64::MAX` sentinel only for
  schedule exhaustion; `ErrorCode` tokens (new domain `0x02xx`) for
  validation failures. Registry test (`ALL_ERROR_CODES`) covers the new
  variants.
- **R5 — Pre-existing leaks acknowledged, not enlarged** (context Box on
  schedule exhaustion; dangling arg1 UB is the established flat-address-space
  trust level of every existing pointer-passing syscall).
- **R6 — Encoder contract pinned by test:** zero args encodes to exactly
  the `(0, 0)` pair; a non-null pointer with zero length is never emitted
  (kernel rejects such half-pairs as malformed).
- **R7 — Kernel blob validation is an exec invariant**, not IPC payload
  interpretation (that rule scopes to `ipc/` message data); the numeric
  limits belong in `system` (contract), the encoding in `usrlib` (boundary
  translation), the argv mechanics in `kernel::task`.
- **R8 — Harness details:** the `spc` fix ships with its own unit-test
  update; the new integration test marks before `send_line` and uses
  `expect_output_since` with generous budgets (args line prints immediately
  at app start).

## Rejected alternatives

- **Opaque blob straight-through with `(ptr, len)` app entry** (the
  `shell-args-to-apps` ABI): rejected by D1 — main already ships the
  standard argc/argv ABI; the blob form would convert all 11 apps and
  fork the ABI a second time.
- **Quoted-argument parser**: deferred (D4); `Command::parse` stays as is.
- **Real argv[0] names at boot sites**: deferred by D2.
- **No limits**: rejected by D3 — unbounded syscall payload acceptance.
