# T3 — usrlib: argument encoding and `load_with_args`

Dependencies: T1, T2. Modifies: `usrlib/src/syscall.rs`.

## Objective

Give user space (and the kernel-linked shell, which uses the same wrapper)
a way to launch an ELF image with arguments over the extended `LoadElf` wire.

## Changes

1. Private encoder (boundary translation lives at the user edge, not in the
   treaty):
   ```rust
   fn encode_arguments(args: &[String]) -> Vec<u8>
   ```
   Canonical blob: each argument's bytes followed by one `0x00`
   (terminator included; `&[]` encodes to an empty `Vec`).
2. Public wrapper next to `load` (which stays byte-for-byte compatible):
   ```rust
   pub fn load_with_args(elf: &'static [u8], args: &[String]) -> Option<UserFutureHandle>
   ```
   - Leaks the `Box<&[u8]>` into `arg1` exactly as `load` does.
   - Encodes args; empty encoding passes `(0, 0)`; otherwise
     `(blob.as_ptr() as usize, blob.len())` — the `Vec` is alive for the
     duration of the synchronous syscall, kernel copies before returning.
   - Returns `UserFutureHandle::from_token(raw)` (both `u64::MAX` and error
     tokens already decode to `None`).

## Acceptance criteria (unit tests in-file)

- `encode_arguments(&[])` is empty, and the wrapper passes exactly
  `(0, 0)` for empty args — pin "never a non-null/zero pair" (review R6).
- `encode_arguments(&[a, b])` == `a\0b\0` byte-for-byte (terminator
  included, no double NUL between args).
- Total encoded size accounting matches the shell's pre-check formula
  (bytes = sum of arg lengths + argument count).
- `cargo test -p usrlib` and `cargo xtask test --skip-integration` green.

Commit: `Encode launch arguments and add load_with_args to the user syscall wrapper`
