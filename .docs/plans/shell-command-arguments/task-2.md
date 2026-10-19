# T2 — Kernel: argv blob validation and task construction from raw blob

Dependencies: T1. Modifies: `kernel/src/task.rs`, `kernel/src/syscall.rs`.

## Objective

Accept a canonical arguments blob on the `LoadElf` path, validate it once
(kernel as single authority), and build the existing `ElfTaskContext` from it.
Boot behavior (`new_elf_task`, placeholder argv `["ELF"]`) must not change.

## Changes

### `kernel/src/task.rs`

1. `ElfTaskContext`: add a constructor that MOVES a caller-validated blob
   into `argument_strings` and reuses `install_argument_pointers()`:
   ```rust
   fn with_blob(elf_bytes: &'static [u8], argument_strings: Vec<u8>) -> Box<ElfTaskContext>
   ```
   Keep `new(elf_bytes, arguments: &[&str])` delegating to `with_blob`
   after joining, if natural.
2. Validation, single entry point used by the syscall arm:
   ```rust
   pub fn validate_arguments_blob(blob: &[u8]) -> Result<(), ErrorCode>
   ```
   Rules (canonical blob, in this precedence):
   - `blob.len() > MAX_ARGUMENT_BYTES` → `Err(ElfArgumentLimitExceeded)`
   - NUL count `> MAX_ARGUMENT_COUNT` → `Err(ElfArgumentLimitExceeded)`
   - empty blob, missing trailing `0x00`, leading `0x00`, or a `0x00 0x00`
     run → `Err(ElfArgumentBlobMalformed)`
   - otherwise `Ok(())`
   The trailing-NUL rule is what makes `install_argument_pointers()`'s
   `unwrap()` provably safe — pin it with a dedicated reject test.
3. New public launch entry:
   ```rust
   pub fn new_elf_task_with_arguments(elf: &'static [u8], arguments_blob: Vec<u8>) -> SharedTask
   ```
   Builds the context via `with_blob`; same `Task::new("ELF", elf_task_wrapper, Box::into_raw(..))`
   pattern. `new_elf_task(elf)` stays untouched (default `&["ELF"]`).
   Do NOT add early returns between `Box::into_raw` and `schedule(..)`.

### `kernel/src/syscall.rs` — LoadElf arm, exact consumption order (review blocker R2)

```rust
Ok(SyscallNum::LoadElf) => {
    if arg1 == 0 { return u64::MAX as usize; }
    let elf_bytes: &[u8] = unsafe { *Box::from_raw(arg1 as *mut &[u8]) };   // consume arg1 FIRST, unconditionally
    if arg2 == 0 && arg3 == 0 { /* today's default path: new_elf_task(elf_bytes) */ }
    if arg3 > MAX_ARGUMENT_BYTES { return ErrorCode::ElfArgumentLimitExceeded.to_reg(); }
    if arg2 == 0 || arg3 == 0 { return ErrorCode::ElfArgumentBlobMalformed.to_reg(); }
    let blob = unsafe { core::slice::from_raw_parts(arg2 as *const u8, arg3) };
    if let Err(code) = validate_arguments_blob(blob) { return code.to_reg(); }
    match kernel().schedule(new_elf_task_with_arguments(elf_bytes, Vec::from(blob))).ok() {
        Some(handle) => handle.pack(),
        None => u64::MAX as usize,
    }
}
```
`Vec::from(blob)` is a synchronous copy — the caller's buffer is dead once
the syscall returns; the context owns the copy. Flat-address-space trust is
the same already established by the `Box::from_raw` arg1 pattern; keep the
`unsafe` block minimal.

## Acceptance criteria (unit tests in-file, `#[cfg(test)]`)

- Accepts canonical `b"a\0b\0c\0"`; context exposes argc 3 and C strings
  `a, b, c` (reuse the existing `read_c_string` test helper).
- Rejects: `b"a\0b"` (no trailing NUL), `b"a\0\0b\0"` (double NUL),
  `b"\0a\0"` (leading NUL), `b""` (empty) → `ElfArgumentBlobMalformed`.
- Rejects 17 arguments and any blob over 512 bytes → `ElfArgumentLimitExceeded`.
- `new_elf_task` default argv remains `["ELF"]` (regression).
- `cargo test -p kernel -- --test-threads=1` green; `cargo build -p rosx` green.

Commit: `Validate argument blobs and build ELF task contexts from them`
