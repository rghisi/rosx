# ROM Filesystem with Built-in Binaries — Implementation Plan

> **Date:** 2026-10-18. **Baseline:** branch `rom-filesystem-vfs` @ `4d75e8c` ("Refactor VFS wire to typed encode/decode methods with retagged replies"), tree clean (verified this session at branch creation).
> **Scope:** `system/` (new codec module + one syscall number), `kernel/` (new `rom` module + one handler arm), `usrlib/` (one wrapper), `arch/x86_64/` (embed + init + build guard), `xtask/` (packer), `apps/vfs_server/` (read-only overlay), `apps/vfs_client/` (ROM checks), `tests-integration/tests/ipc_vfs.rs` (expectation array), `AGENTS.md` (doc sweep). `vfs-protocol/` wire format: **ZERO changes**.
> **User-locked decisions (restate verbatim in every downstream delegation):**
> - **LD1 — one opaque blob, one syscall.** The kernel holds a single ROM image as `&'static [u8]` and exposes exactly one new syscall, `RomRead(offset, buf_ptr, buf_len) -> bytes_copied`. No `RomCount`, no `RomName`, no kernel-side container parsing (LD4), no VFS semantics in the kernel.
> - **LD2 — single ROM image packed by xtask.** After `build_apps()`, xtask packs all 11 `USER_APPS` release ELFs into `target/rosx-user/rom.img` using a minimal container whose layout codec lives in `system/src/rom.rs` (the treaty holds numbers and layouts).
> - **LD3 — flat read-only overlay, zero wire changes.** ROM names appear in `LIST` alongside RAM names; opening a ROM name yields a read-only handle; `MODE_CREATE` on a ROM name → `AlreadyExists`; `WRITE` on a ROM handle → `NotPermitted`; `DELETE` of a ROM name → `NotPermitted`. The 60-byte VFS wire is untouched.
> - **LD4 — boot roster keeps its existing `include_bytes!` blocks** (`arch/x86_64/src/main.rs:74-83`). The kernel rom module is an opaque blob with no codec dependency; the 5 boot ELFs are therefore embedded twice (kernel image duplicates the ROM copy). This duplication is an accepted cost, not a defect.
> - **LD5 — the shell receives ZERO changes.** No `run` command, no removal of shell `include_bytes!`. Run-from-ROM via shell is a later theme.
> - **LD6 — repo law** (AGENTS.md): no comments in any Rust file; commit subjects carry **no prefix**; one commit per step.
> **The plan document itself** is committed separately from all code commits, subject `Add ROM filesystem implementation plan`.

---

## 0. Verified-baseline facts (verified this session against 4d75e8c unless marked `unverified`)

- **Baseline commit/branch:** `4d75e8c` on `rom-filesystem-vfs`; `git status --porcelain` empty at branch creation.
- **The only exhaustive `SyscallNum` consumer:** `kernel/src/syscall.rs:15` — `match SyscallNum::try_from(num) {` (whole-repo grep `match.*SyscallNum|SyscallNum::try_from` over `kernel usrlib system` returns exactly this one match site; usrlib uses variants only as `as usize` values). The match is `#[cfg(not(test))]` (`kernel/src/syscall.rs:13`) — the kernel lib-test target does **not** compile `handle_syscall`, so a new variant's arm is proven only by a kernel build (`cargo xtask build`), never by `cargo test -p kernel`. Binding resolution BR3 makes the variant and the arm land in the same step for this reason.
- **Last syscall number:** `system/src/syscall_numbers.rs:23` — `IpcDataRelease = 26,` (with gaps at 13, 14, 16–19, 21). `RomRead = 27` is free; the plan appends 27 rather than filling a gap, keeping growth monotonic.
- **Syscall ABI shape:** three arguments — `usrlib/src/syscall.rs:15` — `arch::raw_syscall(SyscallNum::Exec as usize, entrypoint as usize, 0, 0);` and `kernel/src/syscall.rs:14` — `pub fn handle_syscall(num: usize, arg1: usize, arg2: usize, arg3: usize) -> usize`. `RomRead` maps `arg1=offset, arg2=buf_ptr, arg3=buf_len` with no packing.
- **Kernel one-shot registry exists:** `kernel/src/once.rs` provides `Once<T>` (`pub const fn new()`, `call_once`, `get`), re-exported `pub mod once;` (`kernel/src/lib.rs:22`), with `unsafe impl<T: Send + Sync> Sync for Once<T>`. `&'static [u8]` is `Send + Sync`.
- **App roster:** `xtask/src/main.rs:5` — `const USER_APPS: [&str; 11] = ["hello_elf", "random_gen_server", "snake", "tetris", "conway", "bulk_echo", "bulk_client", "ping_server", "ping_client", "vfs_server", "vfs_client"];`. Release ELFs land at `target/rosx-user/release/<name>` (built at `xtask/src/main.rs:32-41`; `--debug` affects only the kernel profile, apps are always release).
- **Sorted ROM names (structural derivation from the `USER_APPS` list above, ASCII order):** `bulk_client`, `bulk_echo`, `conway`, `hello_elf`, `ping_client`, `ping_server`, `random_gen_server`, `snake`, `tetris`, `vfs_client`, `vfs_server`. All 11 names are ASCII, unique, 4–17 bytes (≤ 57), and contain no `\n`/`\0` — every name passes `wire::valid_name` (`vfs-protocol/src/wire.rs:77-83`) and the ROM `ROM_MAX_NAME_BYTES` bound.
- **Boot roster block (kept verbatim, LD4):** `arch/x86_64/src/main.rs:74-83` — five `static ..._ELF: &[u8] = include_bytes!("../../../target/rosx-user/release/<app>");` blocks each followed by `let _ = kernel.schedule(new_elf_task(...));`, then `arch/x86_64/src/main.rs:84` — `let _ = kernel.schedule(FunctionTask::new("Shell", shell::shell::main));`. `kernel.setup();` is at `:69`.
- **Build-guard pattern to extend:** `arch/x86_64/build.rs:17-20` — `let elf_path = "../../target/rosx-user/release/random_gen_server";` / `if !Path::new(elf_path).exists() {` / `panic!("missing user ELF {elf_path}; run `cargo xtask apps` first");`.
- **xtask structure:** `run_tests` (`xtask/src/main.rs:75-87`) calls `build_apps()` then `cargo test --workspace`; `build` (`:122-127`) = `build_apps(); build_kernel(debug); build_image(debug, false);`; `run` (`:128-144`) likewise with launch; usage string at `:91` — `"usage: cargo xtask <build [--debug] | run [x86_64] [--debug] | apps | test [--skip-integration]>"`. `xtask/Cargo.toml` `[dependencies]` is **empty** today.
- **Integration harness builds the kernel itself:** `tests-integration/src/qemu.rs:31-34` — `pub fn spawn()` → `kernel_build::kernel_elf()` → `disk_image::create_disk_image(&kernel_elf, ...)`; `tests-integration/src/kernel_build.rs:24` — `let profile = env::var("ROSX_QEMU_PROFILE").unwrap_or_else(|_| "dev".to_string());`, overridable via `ROSX_KERNEL_ELF` (`kernel_build.rs:7`). Consequence: `cargo xtask test` compiles the x86_64 kernel during the test run, so the new `rom.img` build guard is hit **inside** `cargo test` — `run_tests` must pack the ROM before invoking cargo test (step 2 binds this).
- **VFS integration expectations:** `tests-integration/tests/ipc_vfs.rs:15` — `const EXPECTED_OK: [&str; 19] = [`, lines 16–34 in order, ending `"vfs ok: list empty",`; `:13` — `const FORBIDDEN_STRINGS: [&str; 2] = ["fail", "Panic"];`; `:7` — `const BOOT_LINE: &str = "[IPC Server] VFS";`; `:11` — `const DONE_LINE: &str = "vfs done";`. The test asserts **exact count and exact order** of `vfs ok: ` lines, so the `vfs_client` script and this array co-vary as one step (step 5).
- **`vfs_client` list assertions today (the only lines whose comparison logic changes; the printed strings stay byte-identical):** `apps/vfs_client/src/main.rs:124` — `Ok(names) if names == ["notes.txt"] => {}`; `:174` — `Ok(names) if names == ["blob.bin", "notes.txt"] => {}`; `:191` — `Ok(names) if names == ["blob.bin"] => {}`; `:202` — `Ok(names) if names.is_empty() => {}`. With the ROM mounted, `vfs.list()` is never empty — these four comparisons must switch to RAM-name filtering in the same step the mount lands.
- **`vfs_server` dependency graph:** `apps/vfs_server/Cargo.toml` already carries `system = { path = "../../system" }` — step 5 needs **no Cargo change** there. `apps/vfs_server/src/lib.rs` — `#![cfg_attr(not(test), no_std)]` + `extern crate alloc;` + `pub mod fs;`.
- **`RamFs` unit tests today:** `apps/vfs_server/src/fs.rs` contains 28 `#[test]` functions (structural enumeration this session; treat as `unverified` baseline count until the orchestrator's baseline pass confirms the green total).
- **`system` crate shape:** `system/src/lib.rs` — `#![cfg_attr(not(test), no_std)]`, `extern crate alloc;`, mods `syscall_numbers`, `ipc`, `error`. A host-testable `no_std` codec crate with `Vec` output is the established pattern (vfs-protocol precedent).
- **Kernel dependency law:** `kernel/src/lib.rs:6` — `extern crate system;` — using `system::rom` from the kernel would be legal, but per LD4 the kernel rom module uses **nothing** from `system::rom`.
- **CI-proven command forms:** `.github/workflows/ci.yml:31` — `cargo xtask test --skip-integration`; `.github/workflows/ci.yml:51` — `cargo xtask build`. Green-verbatim status on **this** baseline is `unverified` for every command until the orchestrator's baseline pass records it; prior artifact transcripts are not evidence (they describe other trees).
- **Warning fingerprint on this baseline:** none recorded this session → `unverified`. The acceptance warning gate is Δ-new = 0 (location-first) against a fingerprint the baseline pass must capture; no absolute-zero claim is made.
- **ROM/boot ELF duplication:** accepted per LD4; total ROM size = sum of the 11 release ELFs (`unverified` estimate; the packer gate A4 proves a non-trivial size without asserting a count).

---

## 1. Binding resolutions (the coder follows these verbatim)

**BR1 — file law.** The full diff touches exactly: `system/src/lib.rs`, `system/src/rom.rs` (new), `system/src/syscall_numbers.rs`, `kernel/src/lib.rs`, `kernel/src/rom.rs` (new), `kernel/src/syscall.rs`, `usrlib/src/syscall.rs`, `arch/x86_64/build.rs`, `arch/x86_64/src/main.rs`, `xtask/Cargo.toml`, `xtask/src/main.rs`, `apps/vfs_server/src/fs.rs`, `apps/vfs_server/src/main.rs`, `apps/vfs_client/src/main.rs`, `tests-integration/tests/ipc_vfs.rs`, `AGENTS.md`, `Cargo.lock` (step 2 dep edge). Any other path is a defect.

**BR2 — ROM image layout (little-endian throughout).**
```
header  [0..8]    magic = b"ROSXROM1"
        [8..12]   entry_count u32
entries [12..)    entry_count packed records: [name_len u8][name bytes][offset u32][len u32]
blobs   (after    concatenated file data; entry.offset is absolute into the whole image;
 the       )      the blob region starts wherever the packer places it (xtask: immediately
                  after the entry region, no padding)
```
- `system/src/rom.rs` declares, verbatim: `pub const ROM_MAGIC: &[u8; 8] = b"ROSXROM1";` and `pub const ROM_MAX_NAME_BYTES: usize = 57;`.
- Entry ranges satisfy `offset + len <= image_len` (computed in `u64`); names are non-empty UTF-8, ≤ `ROM_MAX_NAME_BYTES`, unique. Offsets/lengths of already-validated entries are surfaced as `usize`.
- `encode` preserves caller order (the packer sorts); `parse` validates eagerly and completely; entries never fail after a successful `parse`.
- The kernel never reads these constants (LD4). The VFS name bound `wire::MAX_NAME_BYTES = 57` remains a VFS-domain rule re-checked by the server at mount; `ROM_MAX_NAME_BYTES` is the ROM-image rule. They coincide at 57 by contract, not by reference.

**BR3 — one-step syscall growth.** `RomRead = 27,` in the `SyscallNum` enum, its `27 => Ok(Self::RomRead),` arm in `TryFrom`, the `Ok(SyscallNum::RomRead) =>` handler arm, and the usrlib wrapper all land in **step 3**, one commit. Adding the variant in any earlier step breaks the kernel build: the match at `kernel/src/syscall.rs:15` is exhaustive over `Result<SyscallNum, ()>` and would gain a non-exhaustive `Ok(RomRead)` pattern (E0004), while host tests compile `handle_syscall` away (`cfg(not(test))`) and would stay falsely green.

**BR4 — `RomRead` semantics.** `handle_syscall`: `arg1 = offset (u64)`, `arg2 = dst pointer`, `arg3 = dst length`; returns bytes copied; `0` means EOF (or no ROM registered); no error code path (mirrors the standing convention that the copy itself cannot fail; bad pointers are the caller's UB — see the unchecked `from_raw_parts` at `kernel/src/syscall.rs:126`). Copy rule: clamp to the registered image; `offset >= image.len()` → 0. An unregistered ROM behaves as a 0-byte image. Old-kernel compatibility: a kernel without number 27 answers unknown numbers `Err(_) => 0` (`kernel/src/syscall.rs:151`), which the server reads as an empty ROM → RAM-only mode. This graceful path is contract, not accident.

**BR5 — mount ordering and fail-fast.** `vfs_server` `main` prints `"[IPC Server] VFS"` **before** mounting (preserves `ipc_vfs.rs:7` `BOOT_LINE` timing), then reads the whole image via `rom_read` in `INCOMING_READ_CHUNK_BYTES` (= 4096) chunks until a chunk returns 0, then: empty image → RAM-only (no attach, no extra line); non-empty → `fs.attach_rom(image)`, on `Err` → `panic!("VFS rom mount error: {err:?}")`. Mount error text must avoid the substrings `fail` and `Panic` (`ipc_vfs.rs:13`) — the `?`-formatted debug must not leak them (the `RomMountError`/`RomError` variant names in BR2/BR8 contain neither).

**BR6 — store-kind check precedes the empty-write shortcut.** In `RamFs::write`, the ROM-kind denial check sits **after** the owner check and file lookup but **before** the current early return `if payload.is_empty() { return Ok(0); }` (`apps/vfs_server/src/fs.rs:83-85`): a zero-byte write to a ROM file is `NotPermitted`, matching real ROM behavior. RAM behavior is byte-for-byte unchanged (the existing `empty_write_is_noop_and_never_resizes` test still passes).

**BR7 — client expectation strings are frozen; only comparisons change.** The 19 existing `vfs ok: `/`vfs error: ` strings keep their exact bytes and order. The four list checks (see baseline facts) switch from exact-vector equality to filtering out `ROM_NAMES` first. Five new checks append after `"vfs ok: list empty"` in this exact order, and `EXPECTED_OK` grows to `&[&str; 24]` with these five appended strings verbatim:
```
vfs ok: rom list complete
vfs ok: rom hello_elf elf magic
vfs ok: rom write denied
vfs ok: rom create denied
vfs ok: rom delete denied
```
`vfs_client` declares `const ROM_NAMES: [&str; 11]` in the sorted order from §0 (derived structurally from the `USER_APPS` line quoted there).

**BR8 — step-internal compilability (consumer-world check).** Every step leaves all worlds buildable: after step 1, `system` compiles everywhere (codec additive, no consumer yet); step 2 compiles only `xtask` (std world, new dep edge); step 3 adds the variant + its only match arm together (BR3) and an unused `pub` usrlib wrapper (no dead-code warning for `pub` lib items); step 4 embeds the blob — kernel builds because step 2 guarantees `rom.img` exists whenever the guard fires (guard message tells bare-builders to run `cargo xtask build`); step 5 co-changes server + client + test array (BR7); step 6 touches docs only. The QEMU integration suite stays green after every step: through step 4 the ROM is embedded but unmounted (server behavior identical); step 5 changes all three co-varying files at once.

**BR9 — no comments, no `unsafe` beyond established convention.** The kernel rom module's only `unsafe` is the existing `from_raw_parts_mut` pattern in the handler arm; `kernel/src/rom.rs` itself is pure Rust over slices. No new comments in any Rust file (LD6).

---

## 2. Steps

### Step 1 — `system`: ROM image codec

**Files:** `system/src/rom.rs` (new), `system/src/lib.rs` (add `pub mod rom;` after `pub mod error;`).
**Why:** the boundary contract for the ROM image must exist before any world (packer or server) reads or writes it.
**Contract:** per BR2, expose:
- `pub const ROM_MAGIC: &[u8; 8] = b"ROSXROM1";`, `pub const ROM_MAX_NAME_BYTES: usize = 57;`
- `#[derive(Debug, PartialEq)] pub enum RomError { TooShort, BadMagic, NameEmpty, NameTooLong, NameNotUtf8, DuplicateName, BeyondImage }`
- `pub struct RomEntry<'a> { pub name: &'a str, pub offset: usize, pub len: usize }`
- `pub struct RomImage<'a>` with `pub fn parse(bytes: &'a [u8]) -> Result<RomImage<'a>, RomError>`, `pub fn files(&self) -> impl Iterator<Item = RomEntry<'a>> + '_`, `pub fn bytes(&self) -> &'a [u8]`
- `pub fn encode(files: &[(&str, &[u8])]) -> Vec<u8>` — panics on `NameEmpty`/`NameTooLong`/`DuplicateName` conditions or on byte offsets exceeding `u32::MAX` (packer-side input control; never called with hostile data)
- `parse` rejects, in prose: fewer than 12 bytes → `TooShort`; magic mismatch → `BadMagic`; entry record running past the end → `TooShort`; `name_len == 0` → `NameEmpty`; `> ROM_MAX_NAME_BYTES` → `NameTooLong`; non-UTF-8 name → `NameNotUtf8`; repeated name → `DuplicateName`; `offset + len > image_len` (u64) → `BeyondImage`. A zero-count image (header + no entries + any trailing bytes) parses with zero entries.
**Tests (`#[cfg(test)]`, byte-pinned house style like `vfs-protocol/src/wire.rs`):** roundtrip name/offset/len and blob bytes; empty-image parse; every rejection above with a targeted malformed byte at its pinned offset; magic pinned to `[b'R', b'O', b'S', b'X', b'R', b'O', b'M', b'1']`.
**Gate:** `cargo test -p system` — `unverified` (single-package host test form; CI only runs workspace invocations). Corroborated by `cargo xtask test --skip-integration`.

### Step 2 — `xtask`: ROM packer

**Files:** `xtask/Cargo.toml` (add `system = { path = "../system" }` to the empty `[dependencies]`; `Cargo.lock` gains the edge), `xtask/src/main.rs`.
**Why:** `rom.img` must exist before any kernel compile (step 4 guard) and before the QEMU suite's own kernel build (baseline fact: `kernel_build.rs` compiles the kernel during `cargo test`).
**Contract:**
- `fn pack_rom()` — for each `USER_APPS` name, `std::fs::read` `workspace_root()/target/rosx-user/release/<name>`; on missing, `eprintln!` + `process::exit(1)` with the message shape `missing user ELF {path}; run `cargo xtask apps` first`; sort the (name, bytes) pairs by name (ASCII); `system::rom::encode`; write `workspace_root()/target/rosx-user/rom.img`; print a `==> packed ROM image {path}` line matching the `run()` banner style (`xtask/src/main.rs:24`).
- Wire into: `build` branch (between `build_apps()` and `build_kernel(debug)`), `run` branch (same position), and `run_tests` (between `build_apps()` and the `cargo test` command) — binding the integration-world dependency named in §0.
- New dispatch arm `"rom"` guarding no flags, patterned on the existing `"apps"` arm (`xtask/src/main.rs:114-121`).
- Update `usage()` (`:91`) to `"usage: cargo xtask <build [--debug] | run [x86_64] [--debug] | apps | rom | test [--skip-integration]>"`.
**Gate:** `cargo xtask rom` (`unverified` — new command), then POSIX: `test -f target/rosx-user/rom.img` and `test "$(wc -c < target/rosx-user/rom.img)" -gt 16384`.

### Step 3 — `system` + `kernel` + `usrlib`: `RomRead` (variant, handler, wrapper — one commit per BR3)

**Files:** `system/src/syscall_numbers.rs`, `kernel/src/syscall.rs`, `kernel/src/rom.rs` (new), `kernel/src/lib.rs` (insert `pub mod rom;` between `pub mod ports;` and `pub mod scheduler;`, preserving the file's alphabetical module list), `usrlib/src/syscall.rs`.
**Contract:**
- `syscall_numbers.rs`: `RomRead = 27,` after `IpcDataRelease = 26,`; `27 => Ok(Self::RomRead),` in `try_from`.
- `kernel/src/rom.rs`:
  - `static ROM_IMAGE: Once<&'static [u8]> = Once::new();` (repo `Once`, `kernel/src/once.rs` — no new `unsafe`)
  - `pub fn init(image: &'static [u8])` → `call_once` (double init panics — single call site in step 4)
  - `pub fn image() -> &'static [u8]` → `get().copied().unwrap_or(&[])` (unregistered = empty; BR4)
  - `pub fn read(offset: u64, dst: &mut [u8]) -> usize` → `read_at(image(), offset, dst)`
  - `pub fn read_at(image: &[u8], offset: u64, dst: &mut [u8]) -> usize` — pure: `offset >= image.len() as u64` → 0 (guard makes the `as usize` cast overflow-safe on both 32- and 64-bit); else copy `min(dst.len(), image.len() - offset)` bytes from `offset`.
  - Host tests (pure `read_at` + one init-then-read test, single `init` call across the process, `--test-threads=1` standing convention): empty image; start-of-image; partial clamp across end; offset past end; `u64::MAX` offset → 0; zero-length dst; `init` + `read` through the registry.
- `kernel/src/syscall.rs`: arm `Ok(SyscallNum::RomRead) =>` building `dst` with `core::slice::from_raw_parts_mut(arg2 as *mut u8, arg3)` (the documented-`unsafe`, unchecked-pointer convention of `IpcDataWrite`/`IpcDataRead`, `:126`/`:136`) and returning `crate::rom::read(arg1 as u64, dst)`.
- `usrlib/src/syscall.rs`: `pub fn rom_read(offset: u64, dst: &mut [u8]) -> usize` → `arch::raw_syscall(SyscallNum::RomRead as usize, offset as usize, dst.as_mut_ptr() as usize, dst.len())`. On x86_32 the `offset as usize` truncation is in-contract: image offsets are `u32`-bounded by BR2 and the `offset >= len` guard runs kernel-side.
**Risks:** `handle_syscall` is `cfg(not(test))` — the arm's compilation is proven only by the kernel build gate below, never by host tests (§0).
**Gate:** `cargo xtask build` (proves kernel + arm + variant + usrlib all compile; `unverified` on this baseline, CI-proven form) and `cargo xtask test --skip-integration`.

### Step 4 — `arch/x86_64`: embed the ROM, init at boot, guard the build

**Files:** `arch/x86_64/src/main.rs`, `arch/x86_64/build.rs`.
**Contract:**
- `main.rs`: immediately after `kernel.setup();` (`:69`) insert
  `static ROM_IMAGE: &[u8] = include_bytes!("../../../target/rosx-user/rom.img");`
  `kernel::rom::init(ROM_IMAGE);`
  (relative depth matches the boot-roster includes at `:74`). Boot roster `:74-83` and `:84` untouched (LD4).
- `build.rs`: add `println!("cargo::rerun-if-changed=../../target/rosx-user/rom.img");` next to the existing rerun lines, and an existence guard in the established style: `let rom_path = "../../target/rosx-user/rom.img";` / `if !Path::new(rom_path).exists() { panic!("missing ROM image {rom_path}; run `cargo xtask build` first"); }`.
- `arch/x86_32`: zero changes — the kernel crate's `rom` module compiles; no init → `image()` empty → `RomRead` answers 0 everywhere.
**Why here:** the guard must ship in the same step as the first `include_bytes!` of `rom.img`; step 2 already packs the image inside `build`/`run`/`test`, so no consumer world can reach a missing image except a bare `cargo build -p rosx`, which now fails with a directive message.
**Gate:** `cargo xtask build`; then full `cargo xtask test` (`unverified`) — **must stay green unchanged**: the ROM is embedded but the server does not yet mount it, so `ipc_vfs` still sees a RAM-only store and its `EXPECTED_OK` array is untouched. This is the regression proof that steps 1–4 changed no observable behavior.

### Step 5 — `vfs_server` overlay + `vfs_client` ROM checks + integration expectations (co-varying, one commit)

**Files:** `apps/vfs_server/src/fs.rs`, `apps/vfs_server/src/main.rs`, `apps/vfs_client/src/main.rs`, `tests-integration/tests/ipc_vfs.rs`.
**fs.rs contract (pure, syscall-free):**
- `pub enum FileStore { Ram(Vec<u8>), Rom { offset: usize, len: usize } }`
- `RamFs` gains fields `rom_blob: Vec<u8>` and `rom_attached: bool`; `RamFs::new()` unchanged in signature; `files: BTreeMap<String, FileStore>` replaces `BTreeMap<String, Vec<u8>>` (all existing sites wrap in `FileStore::Ram` mechanically).
- `#[derive(Debug)] pub enum RomMountError { Format(RomError), InvalidName(String), Duplicate(String), AlreadyAttached }`
- `pub fn attach_rom(&mut self, image: Vec<u8>) -> Result<(), RomMountError>` — `AlreadyAttached` if already attached (never called twice; step 5 main is single-shot); `system::rom::RomImage::parse` failure → `Format`; per entry, `!wire::valid_name(name)` → `InvalidName` (VFS-domain rule enforced server-side); name already present in `files` → `Duplicate`; else insert `FileStore::Rom`. Stores `rom_blob`/`rom_attached` **only on full success** (all-or-nothing).
- `read`: after owner checks, match the store: `Ram` — existing clamp; `Rom { offset, len }` — window `rom_blob[offset..offset+len]` clamped with the same `start = min(offset, len)` / `min(count, len - start)` shape as `fs.rs:66-68`.
- `write`: per BR6, ROM-kind check after lookup, before the empty-payload early return → `Err(VfsError::NotPermitted)`.
- `delete`: after `valid_name`, ROM-kind → `NotPermitted`; RAM path unchanged.
- `open`, `close`, `list`: unchanged (`contains_key` already covers ROM names for `AlreadyExists`/`NotFound`; `list` iterates keys and picks up ROM names automatically, sorted by `BTreeMap`).
- New host tests (fabricated images via `system::rom::encode`, plus a `RamFs` with ROM + RAM collision): list includes ROM names sorted with RAM names; open+read exact bytes; clamped/EOF reads on ROM; open ROM with `MODE_CREATE` → `AlreadyExists`; write (incl. zero-byte) → `NotPermitted`; delete → `NotPermitted`; second `attach_rom` → `AlreadyAttached`; format-invalid image → `Format`; all 28 pre-existing tests pass unchanged.
**main.rs contract (BR5):** `read_rom_image()` loop with `[0u8; INCOMING_READ_CHUNK_BYTES]` chunks via `Syscall::rom_read(offset, &mut chunk)` until 0; `main` order: `println!("[IPC Server] VFS");` → `let mut server = VfsServer::new();` → `server.mount_rom();` → `server.run();`; `mount_rom` skips empty images, panics `!("VFS rom mount error: {err:?}")` on `Err`. Nothing else changes; the serve/dispatch/data paths are ROM-agnostic.
**vfs_client contract (BR7):** `const ROM_NAMES: [&str; 11]` (sorted list from §0); helper filtering a `Vec<String>` list down to non-ROM names; the four comparisons at `:124/:174/:191/:202` switch to filtered equality (`["notes.txt"]`, `["blob.bin", "notes.txt"]`, `["blob.bin"]`, empty) — printed strings byte-identical. Append after `"vfs ok: list empty"`: (1) full list equals `ROM_NAMES` exactly (same order) → `vfs ok: rom list complete`; (2) `open("hello_elf", 0)` + read into a 64-byte buffer returns `Ok(64)` and the buffer starts `[0x7F, b'E', b'L', b'F']` → `vfs ok: rom hello_elf elf magic`; (3) `write` on that same open handle → `Err(ClientError::Vfs(VfsError::NotPermitted))` → `vfs ok: rom write denied`, then `close` succeeds; (4) `open("hello_elf", MODE_CREATE)` → `Err(ClientError::Vfs(VfsError::AlreadyExists))` → `vfs ok: rom create denied`; (5) `delete("hello_elf")` → `Err(ClientError::Vfs(VfsError::NotPermitted))` → `vfs ok: rom delete denied`. Failure strings use the existing `failed(...)` shape and avoid substrings `fail`/`Panic` on success paths.
**ipc_vfs.rs contract:** `const EXPECTED_OK: [&str; 24]` — the 19 existing strings in order, then the five BR7 strings. `BOOT_LINE`, `DONE_LINE`, `FORBIDDEN_STRINGS`, assertions, and timeouts unchanged.
**Gate:** `cargo xtask test --skip-integration`, then `cargo xtask test` (`unverified` on this baseline; harness scripted — `QemuSession::spawn`/`mark`/`expect_output` per baseline facts, so the runtime gate yields machine-countable evidence).
**Blast radius (global-state rule):** processes executing changed code — (a) `vfs_server` lib-test host process (all 28 fs tests + new ROM tests), (b) `vfs_client` ELF under QEMU (new checks), (c) `ipc_vfs` harness process asserting 24 exact ok-lines, (d) `vfs_server` ELF under QEMU (mounted ROM; `BOOT_LINE` ordering per BR5). Processes auditing but unchanged: `ipc_bulk`, `ipc_ping`, `ipc_random`, `boot_and_shell`, `app_lifecycle` — none issue VFS requests or read list output (baseline fact); they must nonetheless run in the same full-suite gate.

### Step 6 — `AGENTS.md` documentation sweep

**Files:** `AGENTS.md`. Correct each invalidated line; quote-current text anchors per guideline:
- Line 33 `│    elf/ ipc/ (mailbox, data) future/ syscall/ keyboard/ task*  cpu (HAL trait)  kconfig` → add `rom/` to the kernel module inventory.
- Line 34 `├── system/      # ABI treaty: syscall numbers, wire layout, error numbers` → `# ABI treaty: syscall numbers, wire layout, ROM image layout, error numbers`.
- Line 53 `cargo xtask build              # ELF apps + kernel + disk image` → `# ELF apps + ROM image + kernel + disk image`; insert `cargo xtask rom               # ROM image from built apps` into that quick-list block (lines 53–55).
- xtask commands block lines 80–82: build comment → `# Build ELF apps + ROM image + x86_64 kernel + disk image`; insert `cargo xtask rom                # Pack target/rosx-user/rom.img from the built user ELFs` after the `apps` line.
- Kernel Subsystems bullet list: add a `rom/` bullet — boot-registered immutable ROM image exposed via the opaque `RomRead` syscall; kernel stays VFS-agnostic, `vfs_server` owns mount semantics.
**Gate:** `grep -E 'cargo xtask rom' AGENTS.md` (non-empty) and `grep -E 'rom/' AGENTS.md` (non-empty).

---

## 3. Acceptance criteria (repo root; every command green verbatim)

- **A0 baseline pass (pre-flight, orchestrator):** at `4d75e8c` capture (i) `cargo xtask test --skip-integration` exit 0 with test totals, (ii) `cargo xtask build` exit 0, (iii) `cargo xtask test` exit 0 with the QEMU roster (10 tests incl. `ipc_vfs`), (iv) the warning fingerprint for the Δ-new gate. All baseline claims in §0 marked `unverified` are resolved here.
- **A1** `cargo xtask test --skip-integration` — exit 0; covers `system::rom` codec tests, `kernel::rom` unit tests, `vfs_server::fs` ROM-mount tests (+ the pre-existing 28 fs tests, count corroborated from A0).
- **A2** `cargo xtask build` — exit 0; proves the `RomRead` arm compiles in the kernel (the only world where `handle_syscall` exists), the build guard fires only with a packed image, and the disk image is produced.
- **A3** `cargo xtask test` — exit 0 **after step 5** with `ipc_vfs::shell_vfs_filesystem_roundtrip ... ok` on exactly 24 `vfs ok: ` lines (machine-countable via the scripted QEMU harness); and, immediately **after step 4**, the same command passes with the unchanged 19-line expectations (step 4's no-observable-change proof).
- **A4** `cargo xtask rom && test -f target/rosx-user/rom.img && test "$(wc -c < target/rosx-user/rom.img)" -gt 16384` — exit 0.
- **A5 grep gates (POSIX):** `grep -E 'RomRead = 27,' system/src/syscall_numbers.rs` non-empty; `grep -E 'pub mod rom;' system/src/lib.rs kernel/src/lib.rs` matches both files; `! grep -nE 'include_bytes!' apps/vfs_server/src/main.rs` (server embeds nothing); LD5 keeps shell's includes — recorded, not violated.
- **A6 warnings:** Δ-new = 0 against the A0 fingerprint, adjudicated location-first (a diagnostic located only in unchanged files is baseline); decreases at legitimately deleted sites are expected, attributed by baseline multiset diff. No absolute-zero claim.

## 4. Risks and edge cases the coder must handle

1. **E0004 trap:** adding `RomRead` to the enum without the handler arm compiles for host tests but breaks the kernel build (`kernel/src/syscall.rs:15` is the sole exhaustive consumer) — BR3 forbids splitting them across steps.
2. **Guard inside `cargo test`:** the QEMU harness compiles the kernel itself at the `dev` profile (`kernel_build.rs:24`); `run_tests` must pack before `cargo test` (step 2 binds this) or `cargo xtask test` fails on the new `build.rs` guard.
3. **Co-variation:** mounting makes `vfs.list()` non-empty, breaking `vfs_client/src/main.rs:202` (`names.is_empty()`) and the three exact-vector list assertions; server, client, and `EXPECTED_OK` land in one commit (BR7/BR8), else the QEMU gate goes red mid-plan.
4. **Empty/unregistered ROM:** x86_32 and old-kernel paths must degrade to RAM-only (`image()` empty → `RomRead` 0 → skip attach); never panic on an empty ROM. A non-empty image that fails validation **must** panic (fail-fast: it is a same-tree build defect).
5. **Pointer ABI:** `dst` is built from user pointers unchecked, the standing convention (`syscall.rs:126` precedent); `offset >= len` guard precedes any `u64 → usize` cast (32-bit overflow-proof).
6. **String hygiene under `FORBIDDEN_STRINGS`:** success-path output must not contain `fail` or `Panic`; mount panic text avoids both (BR5).
7. **Size/heap:** the server heap-holds the whole image (sum of 11 ELFs, `unverified` magnitude, comfortably MB-scale on QEMU); 4096-byte chunk reads mean a few hundred `rom_read` syscalls at boot — well inside `READY_TIMEOUT`/`SWEEP_TIMEOUT`.
8. **ROM self-inclusion:** the image contains `vfs_server` and `vfs_client` binaries (informational; only the kernel embeds the image, so no recursion — kernel size still grows by the ROM plus the boot-roster duplication accepted in LD4).
9. **Cargo.lock:** step 2 adds an xtask→system edge; the lock diff is expected, not a defect (BR1).

## 5. Explicitly out of scope (later themes)

- Shell `run <name>` reading ELFs from the ROM and deletion of `apps/shell/src/shell.rs:10-34` includes + `apps/shell/build.rs` guards (LD5).
- Replacing the boot roster `include_bytes!` with ROM-image reads (LD4 reversal; removes the accepted duplication).
- ROM delivery from disk / Multiboot2 modules instead of the kernel image (blocked on x86_64 by `bootloader` 0.11's lack of module support; x86_32 tag parsing is the natural first step).
- ROM write-back, versioning, or a second image (single image assumed).
