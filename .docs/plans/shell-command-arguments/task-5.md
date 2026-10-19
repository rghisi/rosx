# T5 — Harness: fix space sendkey and script an argument-typing test

Dependencies: T4 (the new test can only pass once the shell forwards).
Modifies: `tests-integration/src/sendkey.rs`,
`tests-integration/tests/shell_command_arguments.rs` (new).

## Changes

1. `tests-integration/src/sendkey.rs`: `token_for(' ')` returns `"spc"`
   (QEMU monitor token; `"sp"` is rejected by QEMU, so scripted lines with
   spaces silently typed nothing today). Update
   `special_characters_map_to_named_tokens` accordingly.
2. New `tests-integration/tests/shell_command_arguments.rs`, mirroring
   `elf_arguments.rs` structure and budgets:
   - `QemuSession::spawn()`, `expect_output("rose>", BOOT_BUDGET = 90s)`
   - `let mark = session.mark();` after boot, then `send_line("pi alpha beta")`
   - `expect_output_since(mark, "args(3): [\"pi\", \"alpha\", \"beta\"]", 30s)`
     (hello_elf prints the argv line before computing Pi digits, so the
     match lands early).

## Acceptance criteria

- `cargo test -p tests-integration -- sendkey` (unit part) green.
- `cargo test -p tests-integration -- --test-threads=1 shell_command_arguments`
  passes; `elf_arguments` still passes.

Commit: `Fix space sendkey token and prove shell argument typing in QEMU`
