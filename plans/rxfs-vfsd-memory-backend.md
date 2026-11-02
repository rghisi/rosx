# RxFS in VFSd — In-Memory Backend — Implementation Plan

> **Date:** 2026-11-01. **Baseline:** `main` @ `10ba20c`, tree clean.
> **Scope:** `apps/vfsd/Cargo.toml`, `apps/vfsd/src/fs.rs` (rewritten), `apps/vfsd/src/main.rs` (type rename only), `protocol/vfs/src/wire.rs` (name validity), `AGENTS.md` sweep, this plan. **The `rxfs` crate is not touched** — it is consumed as-is. No kernel, `system`, `usrlib`, or other-app changes.
> **Nature:** integration theme. `vfsd` keeps its IPC surface exactly as-is; its storage engine swaps from the hand-rolled `BTreeMap<String, Vec<u8>>`-shaped `RamFs` to the real RxFS engine over an 8 MiB `MemoryDevice`. Focus platform x86_64; the change is arch-neutral user-space code, so x86_32 inherits it and must still compile and boot.
> **Decided in discussion (2026-11-01):** full engine swap with ROM materialization; flat namespace, validity fix only (directories are a later theme); fixed 8 MiB volume; `sync()` after every mutating request.

---

## 1. Locked decisions

| # | Decision |
|---|---|
| D1 | **Full swap.** `RamFs` becomes `VfsStore`, owning a `rxfs::volume::FileSystem<rxfs::device::MemoryDevice>`. The `FileStore` enum (Ram/Rom) is deleted — one code path, full engine coverage |
| D2 | **ROM materialization.** At boot (before `run()`), the kernel ROM image is parsed and every ROM file is written into the RxFS volume as a regular file. The daemon keeps a `rom_names: BTreeSet<String>` so writes and deletes of ROM files stay denied exactly as today. Blob is dropped after ingest; peak boot RAM ≈ blob + volume, both trivial |
| D3 | **Flat namespace only.** Wire names map to paths as `/{name}`. `readdir("/")` is the only directory operation; no `Mkdir`/`Rmdir`/`Stat`/path-carrying `List` this theme. rxfs directory errors (`IsADirectory`/`NotADirectory`/`DirectoryNotEmpty`) are unreachable because no directory can be created |
| D4 | **Name validity hardening is mandatory** (the landmine D3 does not dodge): wire `valid_name` additionally rejects `b'/'`, and the whole-names `"."` and `".."`. `decode_name` (server side) enforces `valid_name` after UTF-8 conversion, so the server rejects what no compliant client can encode. Without this, a wire name like `a/b` becomes a path once bridged, and `.`/`..` collide with the root directory's dot entries |
| D5 | **Volume: fixed 8 MiB.** `const VOLUME_BLOCKS: u64 = 2048` (× 4 KiB), `bytes_per_inode = rxfs::spec::DEFAULT_BYTES_PER_INODE` (16 KiB → 512 inodes: ~1 MiB ROM + ample headroom). Constant lives in `fs.rs`, pinned by a unit test. Not derived from ROM size — deterministic geometry beats per-build scaling |
| D6 | **Format, never mount.** `MemoryDevice::new` is zeroed, so no volume ever pre-exists; the daemon formats at boot. UUID is a fixed constant, `volume_name` = `b"rosx-ram"`, `created_at` = 0 (RosX has no clock source; rxfs only stores it). `mount`/`check` remain unwired this theme |
| D7 | **Sync after every mutating wire request.** `Create`-path opens, `Write`, and `Delete` each end with `fs.sync()`; boot ingest ends with one `sync()`. On a memory device sync is a few block copies — cost is nil, the leak-safe commit ordering is genuinely exercised on every op, and `free_blocks`/`free_inodes` stay fresh |
| D8 | **Ownership and mode stay daemon-side.** rxfs has no notion of owners or open modes; the daemon keeps its `opens: BTreeMap<u32, OpenEntry { owner, mode, name, rxfs_handle }>` and its monotonic daemon-issued `u32` wire handles (rxfs `Handle` = slot+generation is never exposed on the wire). rxfs `TooManyOpen` (64 slots, all clients combined) maps to `ServerResource` — today's map is unbounded, this is an accepted cap; dead-client slots leak as they already do |
| D9 | **`list()` sorts.** rxfs `readdir` returns directory-block order; the adapter filters `.`/`..` and sorts byte-wise, preserving today's deterministic BTreeMap order. `ListTooLarge` (> `RPC_MAX_DATA_BYTES`) semantics unchanged |
| D10 | **Error mapping is total, no new `VfsError` codes.** Table in §5 |
| D11 | **Write-path guard order is preserved** (it is what today's unit tests pin): handle lookup + owner check → ROM name denial → empty-payload no-op `Ok(0)` → `MODE_WRITE` check → rxfs write. Zero-fill past EOF is native rxfs behavior (D8 of the crate plan), matching `RamFs::resize` semantics byte-for-byte |

Repo law applies: no comments in Rust files, no `unsafe` (none needed — vfsd never touches the device directly, only `FileSystem<D>` methods), `#[cfg(test)]` modules, no test-only production surface, commit subjects carry no prefix, one commit per step (§8).

---

## 2. Why this shape

- **The engine swap is invisible to clients by construction.** The 60-byte wire, all six opcodes, `Reply` variants, and `client.rs` are untouched; only the two name-validity rules change (D4), and no in-tree name uses `/`, `.`, or `..` (the ROM roster is underscore-constrained by law, the `vfs` app and shell use plain names).
- **`ipc_vfs` going green unmodified is the acceptance proof.** The 19-line scripted roundtrip (create, hole-zeroed write at 8192, EOF, mode denial, reopen persistence, 128 KiB chunked write/read, list, delete-twice denial, final empty list) exercises exactly the rxfs features that replaced the hand-rolled code: allocation + zero-fill, EOF clamping, handle generations, extents, readdir.
- **Daemon is the treaty edge, engine is mechanism.** Encoding name→path, owner/mode enforcement, error collapse, and list shaping all live in `vfsd` — consistent with "translation lives at the boundaries" and the INPUTD precedent (policy in the daemon, mechanism below).

---

## 3. `apps/vfsd/src/fs.rs` — the adapter

Public surface (identical in role to today, consumed only by `main.rs` + own tests):

```
pub struct VfsStore {
    fs: FileSystem<MemoryDevice>,
    opens: BTreeMap<u32, OpenEntry>,     // wire handle -> { owner, mode, name, handle: rxfs Handle }
    next_handle: u32,
    rom_names: BTreeSet<String>,
}

impl VfsStore {
    pub fn new() -> VfsStore                                    // format() per D6
    pub fn attach_rom(&mut self, image: Vec<u8>) -> Result<(), RomMountError>
    pub fn open(&mut self, owner: u32, mode: u8, name: &str) -> Result<u32, VfsError>
    pub fn read(&mut self, owner: u32, handle: u32, offset: u64, count: usize) -> Result<Vec<u8>, VfsError>
    pub fn write(&mut self, owner: u32, handle: u32, offset: u64, payload: &[u8]) -> Result<usize, VfsError>
    pub fn close(&mut self, owner: u32, handle: u32) -> Result<(), VfsError>
    pub fn delete(&mut self, name: &str) -> Result<(), VfsError>
    pub fn list(&self) -> Result<Vec<u8>, VfsError>
}
```

`RomMountError` is retained (`Format(RomError)`, `InvalidName`, `Duplicate`, `AlreadyAttached`) — the validation pass is kept verbatim from today (`wire::valid_name` per entry, duplicate check against `rom_names` + live opens is trivially clean on a fresh volume but keeps its unit tests and guards a second attach; `AlreadyAttached` is enforced by `rom_attached` staying a bool).

### 3.1 Boot: format + ROM ingest

`VfsStore::new()`: `FileSystem::format(MemoryDevice::new(VOLUME_BLOCKS), &FormatParams { bytes_per_inode: DEFAULT_BYTES_PER_INODE, uuid: VOLUME_UUID, volume_name: b"rosx-ram", created_at: 0 })`. Failure `panic!`s with the error (boot-time, unrecoverable, same register as today's `panic!("VFS rom mount error")`).

`attach_rom(image)`:
1. validate: parse `RomImage`, per-entry `wire::valid_name`, duplicate scan — all before touching `fs` (all-or-nothing, as today);
2. for each entry: `fs.create("/{name}")` → `fs.write(handle, 0, &image[offset..offset+len])` (empty files skip the write) → `fs.close(handle)`; record name in `rom_names`;
3. one `fs.sync()`; drop `image`.

`main.rs` is untouched except `RamFs` → `VfsStore` (import, field type, constructor). `mount_rom` keeps its "empty image → return" behavior.

### 3.2 Request mapping (flat bridge, `path(name) = "/" + name`)

| Wire | Adapter action |
|---|---|
| `Open { mode, name }` | `valid_name` already enforced at decode. `fs.lookup(path)`: `Ok(_)` → if `MODE_CREATE` set → `AlreadyExists`, else `fs.open(path)`; `Err(NotFound)` → if `MODE_CREATE` set → `fs.create(path)` + `sync()`, else `NotFound`. On success register `OpenEntry` with daemon handle, owner, mode, rxfs `Handle` (lookup-first mirrors today's control flow and keeps its error precedence) |
| `Read { handle, offset, count }` | owner check; `fs.read(rxfs_handle, offset, &mut buf[..min(count, RPC_MAX_DATA_BYTES)])` → `Vec`. rxfs returns `Ok(0)` at/after EOF → `main.rs` maps to `Reply::Eof` as today. `count == 0` short-circuits in `main.rs` before touching the store (unchanged) |
| `Write { handle, offset, token }` | drain payload in `main.rs` (unchanged); guard order per D11; `fs.write(rxfs_handle, offset, payload)` → `Ok(len)` → `fs.sync()` |
| `Close { handle }` | owner check; `fs.close(rxfs_handle)`; remove adapter entry. A handle to a deleted file still closes cleanly (rxfs slot table holds it until close) — today's behavior, now via generations |
| `Delete { name }` | `valid_name` at decode; `rom_names` → `NotPermitted`; `fs.remove(path)` → `sync()`. Delete-twice → rxfs `NotFound` → `NotFound` (matches today and the integration script) |
| `List` | `fs.readdir(b"/")` → filter `.`/`..` → sort → newline-join → `ListTooLarge` cap |

`sync()` failures (impossible on `MemoryDevice`, total API regardless) map `ServerResource`.

### 3.3 Behavior deltas (accepted, documented here so tests can pin them)

1. **Read/write through an open handle after unlink: `BadHandle`, was `NotFound`.** rxfs invalidates on freed inode; the handle table slot is still valid until `close`. Only the `fs.rs` unit suite covers this today (`delete_then_read_via_still_open_handle_is_not_found` → renamed and re-anchored to `BadHandle`); no integration step touches it.
2. **64 simultaneous opens across all clients** (`rxfs::spec::MAX_OPEN`); overflow → `ServerResource`. Real usage is one open file per shell launch.
3. **ROM files are physical copies** inside the volume (byte-identical content, `NotPermitted` on write/delete via the name-set, same wire-visible behavior).
4. **Names containing `/`, `.`, `..` are rejected** at `encode` and at `decode` (D4) — previously legal-but-useless wire names.

---

## 4. `protocol/vfs/src/wire.rs` — validity fix

- `valid_name`: add `!bytes.contains(&b'/')` and reject the whole-names `.` and `..`. Byte length / no-`\n` / no-`\0` rules unchanged; cap stays 57.
- `decode_name`: after `from_utf8`, run `valid_name` on the result; failure → `InvalidPath`.
- Wire tests: extend `valid_name_rejects_empty_oversize_and_forbidden_bytes` with `/`, `.`, `..`; new decode tests pinning `Open`/`Delete` frames carrying `a/b`, `.`, `..` → `Err(InvalidPath)`; existing byte-pinned frames unaffected.
- `client.rs` unchanged (it calls `encode`); `shell`, `vfs` app, integration expectations unchanged.

---

## 5. Error mapping (D10, total)

| `rxfs::error::FsError` | `VfsError` |
|---|---|
| `NotFound` | `NotFound` |
| `AlreadyExists` | `AlreadyExists` |
| `InvalidPath`, `PathTooLong`, `NameTooLong`, `InvalidName` | `InvalidPath` |
| `BadHandle` | `BadHandle` |
| `Io(_)`, `Corrupt`, `DeviceTooSmall`, `VolumeTooLarge`, `DepthExceeded`, `NotADirectory`, `IsADirectory`, `DirectoryNotEmpty`, `NoSpace`, `NoInodeSpace`, `FileTooLarge`, `TooManyOpen` | `ServerResource` |

The collapsed directory/geometry errors are unreachable in the flat phase (no directory can be created, geometry is boot-fixed and pre-validated); the mapping is total so no `FsError` can escape the adapter as a panic or a wrong code.

---

## 6. Memory budget

8 MiB volume + 64-slot metadata cache (256 KiB) + bitmaps (256 B + 64 B) + ~1 MiB ROM blob held transiently. All from the daemon heap via `Syscall::alloc` (large sizes fall through to the kernel `FreeList`); QEMU boots with its 128 MiB default (the runner passes no `-m`). The 8 MiB contiguous first-touch replaces the current lazy per-file `Vec` growth — integration A4 is the proof it lands at boot, before the ROM roster (and `shell`) starts asking for files.

## 7. Test strategy

House style: `#[cfg(test)]` in `fs.rs`, same OWNER/INTRUDER fixtures. The existing suite is re-anchored, not deleted: every present expectation survives except delta #1 (§3.3), and each test now runs against the real engine.

1. **Carry-over, unchanged expectations:** create/first-handle-1, reopen persistence, create-existing → `AlreadyExists`, open-missing → `NotFound`, handle monotonicity, name validation at the store boundary, EOF/clamp/hole-zeroing, empty-write no-op, mode-zero write denial, cross-owner `BadHandle` matrix, close twice, delete-twice, handle-to-deleted-file closes cleanly, list ordering + `ListTooLarge`, all six ROM tests (attach lists ROM+RAM sorted, exact reads, window clamp/EOF, create-over-ROM `AlreadyExists`, ROM write denial incl. empty payloads, ROM delete denial, second attach `AlreadyAttached`, invalid-image, invalid-name rejection, collision rejection stores nothing).
2. **New pins:** `VOLUME_BLOCKS == 2048` and `Geometry::plan` invariants for it (≥ 512 inodes, root writable); `.`/`..` filtered from `list` (root carries dot entries from `format`); read-after-unlink → `BadHandle`; 64-open cap → `ServerResource` on the 65th open (loop-create on distinct names; close one → open succeeds again); ROM ingest is byte-exact for a multi-file fabricated ROM (reuse `fabricated_rom()` + an ELF-sized payload).
3. **Wire tests** per §4.
4. **Integration:** `ipc_vfs` on x86_64 unmodified (A4). It is the drop-in proof; nothing else changes.

## 8. Steps (one commit each; every step green before the next)

| # | Deliverable | Gate |
|---|---|---|
| 1 | `protocol/vfs`: `valid_name` + `decode_name` hardening (§4) + wire tests | `cargo test -p vfs-protocol` |
| 2 | `apps/vfsd`: add `rxfs = { path = "../../fs/rxfs" }`; rewrite `fs.rs` as `VfsStore` (§3) with ROM ingest; `main.rs` type rename; full `fs.rs` suite re-anchored | `cargo test -p vfsd && cargo build -p vfsd` |
| 3 | `AGENTS.md`: `fs/rxfs` bullet drops "not yet wired into the OS" and names the vfsd memory backend; apps line for `vfsd` mentions RxFS + `MemoryDevice`; D5/D7 recorded in the Disk/filespace area of Kernel Subsystems | grep `rxfs` context reads true in `AGENTS.md` |
| 4 | Full verification sweep (§9) — lands only if it finds fixes | gates below |

## 9. Acceptance (repo root, all green)

- **A1** `cargo test -p vfs-protocol && cargo test -p vfsd && cargo test -p rxfs` — exit 0.
- **A2** `cargo xtask test --skip-integration` — exit 0 (workspace unit suite, `rxfs` default-member included).
- **A3** `cargo xtask build` and `cargo xtask build x86_32` — exit 0 (the rxfs bare-target build for both user targets is already proven by the crate plan's A3; this re-proves it through the real dependency edge).
- **A4** `cargo test -p tests-integration -- --test-threads=1` on x86_64 — `ipc_vfs` green unmodified; remaining suites unaffected (they do not touch VFS storage).
- **A5** `ROSX_ARCH=x86_32 cargo test -p tests-integration -- --test-threads=1` — exit 0 (`ipc_vfs` is x86_64-only; the x86_32 suites boot the same vfsd binary — the volume formats and the shell still launches by name).
- **A6** Δ-new warnings = 0 vs baseline.

## 10. Explicitly out of scope (later themes)

- Directory support on the wire: `Mkdir`/`Rmdir`/`List{path}`/`Stat`, path-carrying `Open`/`Delete`, name cap growth 57→255, and honest `IsADirectory`/`NotADirectory`/`DirectoryNotEmpty`/`TooManyOpen` error codes.
- `rename()`/`truncate()` exposure; symlinks; permission enforcement from stored modes.
- The ATA `BlockDevice` adapter and an `fsd`-style daemon mounting a real disk (`QemuDevice`/atad-backed); volume growth beyond 8 MiB; `mount`-or-format boot; `check()` exposed to `kstatsd`-style observability; `xtask fsimg`.
- Wire-observable clock (`mtime` stays a txg stamp), ROM-less boot paths.

## 11. Risks and accepted costs

1. **`MAX_OPEN` = 64 total open handles** replaces an unbounded table; dead clients leak slots today too. Accepted; the cap is uniform and maps cleanly to `ServerResource`.
2. **8 MiB eager allocation at boot** is the largest single user-space allocation in the OS today. One contiguous `Syscall::alloc` into the kernel `FreeList`; A4/A5 prove it. Fallback knob: drop `VOLUME_BLOCKS` to 1024 (4 MiB) — still 4× ROM headroom.
3. **Boot-time ROM ingest** adds ~256 block writes + metadata commits — microseconds-to-milliseconds on `MemoryDevice`, invisible against kernel boot.
4. **`BadHandle`-after-unlink** is a wire-observable change (delta #1). Chosen over adapter-side name-table gymnastics because generation invalidation is the engine's correctness story; nothing in-tree consumes the old code on that path.
5. **ROM content duplication** (blob dropped post-ingest, but content is copied once into volume blocks): ~1 MiB steady-state instead of zero-copy windows. The price of one code path (D1/D2).
