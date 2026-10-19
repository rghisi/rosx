# T1 — Treaty: argument limits and error codes

Dependencies: none. Modifies: `system/src/args.rs` (new), `system/src/lib.rs`, `system/src/error.rs`.

## Objective

Put the boundary contract for launch arguments in `system`, so the kernel,
usrlib, and shell share one source of truth.

## Changes

1. `system/src/args.rs` (new module, constants only — contract, not mechanism):
   ```rust
   pub const MAX_ARGUMENT_COUNT: usize = 16;
   pub const MAX_ARGUMENT_BYTES: usize = 512;
   ```
   Style precedent: `system/src/ipc.rs` (`MESSAGE_BYTES`).
2. `system/src/lib.rs`: add `pub mod args;`.
3. `system/src/error.rs`: open a new error domain for ELF launch:
   ```rust
   ElfArgumentLimitExceeded = 0x0201,
   ElfArgumentBlobMalformed = 0x0202,
   ```
   - Add both `from_code` match arms.
   - Extend `ALL_ERROR_CODES` registry array from 13 to 15 entries (before
     `Unknown` stays at the end wherever the array orders it).

## Acceptance criteria

- `cargo test -p system` green, including
  `every_error_code_variant_roundtrips_through_the_register_codec`.
- `cargo xtask test --skip-integration` green (nothing else changed behavior).

Commit: `Add argument count and byte limits plus ELF launch error codes to the treaty`
