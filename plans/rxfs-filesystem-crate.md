# RxFS — Standalone Filesystem Crate — Implementation Plan

> **Date:** 2026-11-01. **Baseline:** `main` @ `62c530d`, tree clean.
> **Scope:** new crate `fs/rxfs/` + workspace wiring (`Cargo.toml`, `Cargo.lock`) + this plan + `AGENTS.md` sweep. **No other path is touched** — no kernel, `system`, `usrlib`, `protocol/`, or app changes. Integration (`fsd` daemon, `vfsd` backend swap, wire growth) is a later theme (§10).
> **Nature:** a pure `no_std` library crate: on-disk format + engine + tests. Zero dependencies beyond `core`/`alloc`; zero dependency on `kernel`, `system`, `usrlib` (and zero third-party). It is a mechanism, not a treaty, so it does **not** live under `protocol/` and nothing links it in the kernel.

---

## 1. Locked decisions (from the design discussion — the coder follows these verbatim)

| # | Decision |
|---|---|
| D1 | Block size **fixed 4096** bytes (`BLOCK_BYTES` constant; superblock carries the field defensively, mount rejects any other value) |
| D2 | **u32 block addressing** — volume ceiling 2³² × 4 KiB = 16 TiB; extents stay 8 bytes |
| D3 | **Static inode table** (ext2-shaped): inode n lives at a fixed offset, O(1), no indirection |
| D4 | **Checksums on superblock + inodes only** (Fletcher-32, ZFS-style); extent/dir blocks are validated structurally, not checksummed, in v1 |
| D5 | **No journal**; leak-safe commit ordering (§7.3) + `check()` fsck-lite repair; clean/unclean mount flag; backup superblock |
| D6 | **`rename()` and `truncate()` deferred** to v2 |
| D7 | **Names byte-transparent** (not validated as UTF-8), case-sensitive by raw byte equality, max **255 bytes** — 255 chosen over 256 because `name_len` fits `u8`, making the dir-entry header exactly 8 bytes (ext2-shaped). "Up to 256 chars" is satisfied for any single-byte charset; documented |
| D8 | **No holes in v1**: writes past EOF zero-fill; every byte ≤ size is backed by an allocated block |
| D9 | **Metadata block cache** in the mounted filesystem (fixed 64-slot direct-mapped write-back, §7.2); file data streams through uncached |
| D10 | Placement **`fs/rxfs/`** — new top-level `fs/` dir so future filesystem types sit beside it. Workspace member **and** `default-members` entry |
| D11 | Path cap **4096 bytes** total; component cap 255; component-depth cap 512 |

Repo law applies (D-law): no comments in any Rust file, no `unsafe` (there is none in this design), unit tests per module in `#[cfg(test)]`, no test-only production surface, commit subjects carry no prefix, one commit per step.

---

## 2. Inspiration map

| Source | Taken | Refused |
|---|---|---|
| ext2/ext4 | superblock + static inode table + block bitmap + inode bitmap; `rec_len`-chained dir entries with `inode=0` holes; extents (simplified); clean/unclean flag; `bytes_per_inode` format param | block groups, journal, xattrs, htree dirs |
| XFS | extents-first data model; allocation hint for locality; AGs noted as the scaling path | B+-trees everywhere, the log |
| ZFS | checksummed superblock with txg commit counter + backup-superblock fallback; Fletcher-32 | COW, pools, snapshots |
| NTFS | variable-length directory-entry names (their 255-char cap motivates ours) | MFT indirection, attributes |
| FAT32 | one simple volume-wide allocator; whole-FS-fits-in-your-head vibe | fixed 32-byte dir slots (LFN chains), total absence of integrity discipline |

---

## 3. On-disk format

Little-endian throughout. All "blocks" are 4096 bytes. LBA/device-relative: the FS consumes `dev.block_count()` from block 0; partition/offset concerns belong to whatever adapter implements `BlockDevice`.

### 3.1 Volume geometry

```
block 0                 reserved (future MBR / boot structure; never touched)
block 1                 Superblock (primary)
blocks 2..=7            reserved pad (zero; future journal slot)
block 8 ..              Block Bitmap      — 1 bit per block, 1 = used
            ..          Inode Bitmap      — 1 bit per inode, 1 = used
            ..          Inode Table       — inode_count × 256 B, fixed offsets
            ..          DATA REGION       — every block reachable only via extents
last block              Backup Superblock
```

`Geometry::plan(total_blocks, bytes_per_inode)` (pure fn, fully unit-tested) derives everything:

```
inode_count          = max(64, (total_blocks × 4096 / bytes_per_inode) rounded DOWN to a multiple of 16)
block_bitmap_blocks  = ceil(total_blocks      / (8 × 4096))
inode_bitmap_blocks  = ceil(inode_count       / (8 × 4096))
inode_table_blocks   = inode_count / 16            (256 × 16 = 4096, exact)
data_start           = 8 + block_bitmap_blocks + inode_bitmap_blocks + inode_table_blocks
usable_data_blocks   = total_blocks − 1 − data_start          (must be ≥ 32 else DeviceTooSmall)
```

Default `bytes_per_inode` = 16384 (ext2 default): 1 GiB volume → 65 536 inodes → 16 MiB inode table, 32 KiB block bitmap. Bitmap bits beyond `total_blocks` (and inode bits beyond `inode_count`) are **preset used** so allocation never walks off the end. Format presets every metadata bit (blocks 0..data_start and the backup-SB block) used.

### 3.2 Superblock — block 1 and the last block, bytes 0..168 meaningful

| Off | Field | Notes |
|---|---|---|
| 0 | magic `[u8;8]` | `b"RXFSV1__"` |
| 8 | version `u32` | 1 |
| 12 | block_size `u32` | 4096 |
| 16 | total_blocks `u64` | ≤ 2³² |
| 24 | inode_count `u32` | multiple of 16 |
| 28 | inode_size `u16` | 256 |
| 30 | flags `u16` | 0 |
| 32 | block_bitmap_start `u64` | |
| 40 | block_bitmap_blocks `u32` | |
| 44 | inode_bitmap_start `u64` | |
| 52 | inode_bitmap_blocks `u32` | |
| 56 | inode_table_start `u64` | |
| 64 | inode_table_blocks `u32` | |
| 68 | reserved `u32` | 0 |
| 72 | root_inode `u32` | 1 |
| 76 | mount_state `u8` | 0 clean / 1 unclean |
| 77 | future `u8` | 0 |
| 78 | reserved `u16` | 0 |
| 80 | volume_uuid `[u8;16]` | caller-supplied at format |
| 96 | volume_name `[u8;32]` | caller-supplied bytes, NUL-padded, byte-transparent |
| 128 | created_at `u64` | caller-supplied (no clock source in RosX yet; 0 allowed) |
| 136 | txg `u64` | commit counter, +1 per `sync()` |
| 144 | free_blocks `u64` | advisory; mount recomputes from bitmaps |
| 152 | free_inodes `u32` | advisory; mount recomputes |
| 156 | alloc_hint `u32` | persisted best-effort |
| 160 | checksum `u32` | Fletcher-32 over the whole 4096-byte block with this field zeroed |
| 168..4096 | zero | |

Checksum rule (uniform for superblock and inodes): **computed over the whole unit with the checksum field's bytes treated as zero.** No ambiguity for codec tests.

### 3.3 Inode — 256 bytes, inode n at `inode_table_start × 4096 + n × 256`

| Off | Field | Notes |
|---|---|---|
| 0 | checksum `u32` | Fletcher-32 over the 256 bytes, field zeroed |
| 4 | kind `u8` | 0 = free, 1 = regular, 2 = directory |
| 5 | flags `u8` | 0 |
| 6 | mode `u16` | stored, never enforced (0o644 files / 0o755 dirs at create) |
| 8 | links `u32` | file: 1; dir: 2 + number of immediate subdirs |
| 12 | generation `u32` | bumped when the slot is freed; distinguishes recycled slots |
| 16 | size `u64` | bytes; directories: always a multiple of 4096 |
| 24 | mtime `u64` | txg value at last mutation (wall clock later; honest placeholder) |
| 32 | blocks_used `u32` | sum of extent lengths |
| 36 | extent_count `u16` | total extents; the first 26 are inline |
| 38 | spill_block `u32` | first extent-list block, 0 = none |
| 42 | reserved `[u16;3]` | 0 |
| 48 | inline extents | 26 × `{ start u32, length u32 }` (48 + 208 = 256) |

Extent record: 8 bytes, `{ start u32, length u32 }`. `start` is never 0 (block 0 is reserved). Invariant: extents are ordered by file offset and **no two extents of one file are adjacent** (adjacent ones are coalesced; `check()` flags violations). A free inode is kind=0 with `generation` preserved and everything else zero — its checksum is recomputed accordingly.

### 3.4 Extent-list block (spill, D2/D4: no checksum in v1)

```
0  next_block u32        0 = end of chain
4  count u16             ≤ 511
6  reserved u16          0
8  extents 511 × 8 B = 4088 → exactly 4096
```

A file with `extent_count > 26` keeps the first 26 inline and the remainder, in order, across the spill chain. Chain capacity is unbounded in practice.

### 3.5 Directory entry (inside directory data blocks)

```
0  inode u32         0 = deleted hole
4  rec_len u16       multiple of 4, ≥ entry size; distance to next entry in THIS block
6  name_len u8       1..=255
7  kind u8           1 regular / 2 directory (mirrors the inode kind for cheap listing)
8  name [u8; name_len]

entry_size = (8 + name_len + 3) & !3          (min 12, max 264 for a 255-byte name)
```

Rules (all unit-tested against a `&mut [u8; 4096]` block):
- An entry never straddles a block. Block sizes are multiples of 4, so the last live entry of a block carries the tail: when an entry doesn't fit the remainder, the remainder is absorbed into the last entry's (or trailing hole's) `rec_len` and the entry lands in the next block.
- Directory data length is always a whole number of blocks; the dir `size` field is `blocks × 4096`.
- Delete: if a live previous entry exists in the same block, absorb (`prev.rec_len += this.rec_len`); else mark `inode = 0` (hole). Create: first-fit a hole with `rec_len − 8 ≥ 1 + ...` (split when leftover ≥ 12: entry + new hole); else append to block free space; else extend the directory with a new block.
- `readdir` skips holes (index counts live entries).
- Every directory holds `.` (self) and `..` (parent; in root, root itself).
- Lookup: linear scan, **raw byte equality** (case-sensitive), no normalization.

### 3.6 Names and paths (API law, `path.rs`)

- Absolute, `/`-separated; leading `/` required; trailing `/` tolerated; empty component → `InvalidPath`.
- Component: 1..=255 bytes; rejects `0x00` and `/`. Root component resolution via `.`/`..` entries.
- Total path ≤ 4096 bytes; resolved depth ≤ 512 (cycles are structurally impossible without symlinks; the cap guards against corrupted `..` loops during `check()` walks).
- Names are `[u8]` everywhere in the public API — byte-transparent (D7). Validity is enforced once, here.

### 3.7 Limits summary

| Thing | Cap |
|---|---|
| Volume | 16 TiB (u32 block × 4 KiB) |
| Filename | 255 bytes (D7) |
| Path | 4096 bytes; depth 512 |
| Inodes | ≤ 2³², fixed at format |
| File | 26 inline extents + chained spill; ≤ 16 TiB |
| Entries per directory | unbounded (grows via its own extents) |
| Open handles | 64 per mounted volume (`MAX_OPEN`) |

---

## 4. Crate shape

```
fs/rxfs/Cargo.toml        package "rxfs", edition.workspace, [dependencies] EMPTY, [features] std = []
fs/rxfs/src/
  lib.rs        #![no_std] + extern crate alloc; pub mods: checksum, device, dir, error, extent,
                inode, path, spec, superblock, volume
  spec.rs       constants: BLOCK_BYTES 4096, FS_MAGIC b"RXFSV1__", VERSION 1, INODE_SIZE 256,
                INLINE_EXTENTS 26, SPILL_EXTENTS 511, MAX_NAME_BYTES 255, MAX_PATH_BYTES 4096,
                MAX_DEPTH 512, MAX_OPEN 64, RESERVED_PREFIX_BLOCKS 8, ROOT_INODE 1,
                DEFAULT_BYTES_PER_INODE 16384, geometry plan fn
  error.rs      IoError, FsError
  device.rs     BlockDevice trait, MemoryDevice
  checksum.rs   fletcher32
  superblock.rs Superblock codec
  bitmap.rs     in-RAM bit ops + run allocator + region load/store
  extent.rs     Extent, ExtentList (inline 26 + spill chain), coalescing
  inode.rs      Inode 256-B codec
  dir.rs        entry codec + block-stream ops (pure over block slices)
  path.rs       validation + component iterator
  cache.rs      MetaCache (64-slot direct-mapped write-back)
  volume.rs     FileSystem<D>: format/mount/ops/sync/check
  file.rs       (feature = "std") FileDevice over std::fs::File
```

Host tests use `#[cfg(test)]` (std available in the test harness, the established `#![cfg_attr]`-free pattern is fine here: the lib itself is plain `#![no_std]`; tests run on host). `FileDevice` is the only `#[cfg(feature = "std")]` item.

### 4.1 Device trait and backends

```rust
pub enum IoError { Device, Flush }

pub trait BlockDevice {
    fn block_count(&self) -> u64;
    fn read_block(&mut self, block: u64, buf: &mut [u8]) -> Result<(), IoError>;   // buf.len() == 4096
    fn write_block(&mut self, block: u64, buf: &[u8]) -> Result<(), IoError>;
    fn flush(&mut self) -> Result<(), IoError>;
}
```

- `MemoryDevice` (in-crate, `alloc`-backed `Vec<u8>`): the in-memory backend for tests and any consumer; also records an op log under `#[cfg(test)]` harnesses live in test modules, not in it.
- `FileDevice` (`std` feature): host file, seek/read/write at 4096 granularity — "test everything as a file on the dev host".
- Raw block device (ATA): a ~30-line adapter **in the future `fsd` daemon** over the `ata-protocol` client (8 sectors per FS block, chunks ≤ 128 sectors = 16 blocks). Not part of this crate — that's what keeps deps at zero.

### 4.2 Public API (volume.rs)

```rust
pub enum Kind { Regular, Directory }                      // u8 codes 1, 2

pub struct FormatParams<'a> { pub bytes_per_inode: u32, pub uuid: [u8; 16],
                              pub volume_name: &'a [u8], pub created_at: u64 }

pub fn format<D: BlockDevice>(dev: &mut D, params: &FormatParams) -> Result<FileSystem<D>, FsError>
pub fn mount<D: BlockDevice>(dev: &mut D) -> Result<FileSystem<D>, FsError>

#[derive(Clone, Copy, PartialEq)] pub struct Handle { slot: u8, generation: u32 }

pub struct Stat { pub inode: u32, pub generation: u32, pub kind: Kind,
                  pub size: u64, pub blocks: u32, pub mtime: u64 }

#[derive(Clone, Copy)] pub struct DirItem { pub inode: u32, pub kind: Kind,
                                             pub name: [u8; 255], pub name_len: u8 }

pub struct Report { pub leaked_blocks: u64, pub leaked_inodes: u64, pub dangling_entries: u64,
                    pub double_refs: u64, pub fixed: bool, pub hard_corrupt: bool }

impl<D: BlockDevice> FileSystem<D> {
    fn unclean_previous(&self) -> bool
    fn lookup(&mut self, path: &[u8]) -> Result<Stat, FsError>
    fn open(&mut self, path: &[u8]) -> Result<Handle, FsError>
    fn create(&mut self, path: &[u8]) -> Result<Handle, FsError>
    fn mkdir(&mut self, path: &[u8]) -> Result<(), FsError>
    fn read(&mut self, h: Handle, offset: u64, buf: &mut [u8]) -> Result<usize, FsError>
    fn write(&mut self, h: Handle, offset: u64, data: &[u8]) -> Result<usize, FsError>
    fn readdir(&mut self, h: Handle, index: u32) -> Result<Option<DirItem>, FsError>
    fn stat(&mut self, h: Handle) -> Result<Stat, FsError>
    fn close(&mut self, h: Handle) -> Result<(), FsError>
    fn remove(&mut self, path: &[u8]) -> Result<(), FsError>
    fn rmdir(&mut self, path: &[u8]) -> Result<(), FsError>
    fn sync(&mut self) -> Result<(), FsError>
    fn unmount(self) -> Result<D, FsError>
    fn check(&mut self) -> Result<Report, FsError>
}

pub enum FsError { Io(IoError), DeviceTooSmall, VolumeTooLarge, Corrupt, NotFound, AlreadyExists,
                   NotADirectory, IsADirectory, DirectoryNotEmpty, NameTooLong, PathTooLong,
                   InvalidName, InvalidPath, NoSpace, NoInodeSpace, FileTooLarge, BadHandle,
                   TooManyOpen, DepthExceeded }
```

Semantics: `create`/`mkdir` require an existing directory parent and a non-existent name; `open` accepts files and directories (dirs opened for `readdir` only — `read`/`write` on a dir handle is `IsADirectory`); `write` at/after EOF zero-fills the gap and grows (D8); `remove` on a directory is `IsADirectory`; `rmdir` requires emptiness beyond `.`/`..`; stale handles fail `BadHandle` via generation match; no `rename`, no `truncate` (D6). Single-writer by construction: everything takes `&mut self`, no locks anywhere.

---

## 5. Mount / format semantics

- **format**: validate device size vs `Geometry::plan` → zero the reserved prefix → write both bitmap regions (metadata bits set, spare bits set) → zero the inode table → write root inode #1 (directory, links 2, its single block containing `.`/`..`) and mark root's block used → write root dir entry chain → write primary superblock (txg 0, clean) → write backup superblock → `flush`. `format` returns a mounted, clean `FileSystem`.
- **mount**: read block 1; magic+version+checksum OK → use primary. Else read the last block; valid → use backup and force `unclean_previous = true`. Else `Corrupt`. Validate geometry against `dev.block_count()` (`total_blocks` > device → `Corrupt`; device larger is tolerated, the tail is unused). Load both bitmaps; **recompute `free_blocks`/`free_inodes` from popcount** (SB counters are advisory — drift-tolerant by design). Write the primary superblock back with `mount_state = unclean` (the crash marker). Caller is expected to run `check()` when `unclean_previous()`.
- **unmount**: sync with clean flag (both superblocks), `flush`, return the device.

---

## 6. Allocation policy

- **Block runs**: hinted first-fit over the in-RAM bitmap — start at `alloc_hint`, find the first free bit (wrap once), then extend forward to the **longest contiguous run** up to the requested cap. Hint advances past the run. Long runs keep extent counts near 1 for sequential writes (XFS locality idea at ext2 simplicity).
- **Inode runs**: inode bitmap first-fit with hint.
- **Locality**: data hint seeded from the parent directory inode's first extent block when creating; global hint otherwise.
- **Growth write at EOF**: allocate `ceil((offset+len)/4096) − blocks_used` blocks in runs; if the file's last extent ends where a new run starts (and `length` fits `u32`), extend that extent in place instead of appending (coalescing); else append inline, else spill.
- **Free**: clear bits per extent, in run units; inode freed by writing kind=0 with `generation += 1` and a fresh checksum.

---

## 7. Engine internals

### 7.1 Mounted state

```
dev, layout (Geometry), sb (RAM copy), block_bitmap: Vec<u8>, inode_bitmap: Vec<u8> (+dirty),
cache: MetaCache, opens: [Option<OpenSlot>; 64]  (OpenSlot { inode_no, generation }),
alloc_hint, dirty_sb
```

The open table stores only `(inode_no, generation)` — the **cache is the single source of truth for inode contents**; every mutation goes `inode_mut(no, |i| ...)` → cache block read-modify-write → slot dirty. No parallel copies of truth.

### 7.2 MetaCache (D9)

Fixed 64 slots × (tag u64 + dirty flag + 4096 B) ≈ 263 KiB. **Direct-mapped** (`slot = block % 64`): load-on-miss evicts a dirty victim by writing it back. Ops: `get`, `mark_dirty`, `invalidate`, `write_back(block)`, `flush_all`. Caches **metadata blocks only**: inode-table blocks, directory data blocks, extent-list blocks. File data blocks stream direct to the device — they never enter the cache, so no aliasing is possible (and `read`/`write` on directory handles are rejected anyway). Pathological conflict thrashing is the accepted v1 cost; 2-way/LRU is the recorded upgrade path.

### 7.3 Leak-safe commit ordering (D5) — the central correctness contract

**Invariant: a crash may leak space, never corrupt or overwrite in-use data.**

| Operation | On-disk order | Crash leaves |
|---|---|---|
| allocate run | bitmap bits set (flushed no later than the referencing sync) | leaked-but-marked blocks (safe) |
| `create`/`mkdir` | inode written → **then** dir entry | orphan inode (no entry), or fully intact |
| data write | data block written (immediate) → inode updated at commit | new bytes lost, file consistent |
| `remove`/`rmdir` | dir entry removed → inode kind=0 → **bitmap freed last** | dangling entry → invalid inode (reclaimable); marked-used unreferenced blocks (leak) |
| `sync()` | 1. flush dirty cache blocks → 2. write bitmap regions → 3. update SB (counters from popcount, `txg += 1`, hint) → 4. write primary + backup SB → 5. `dev.flush()` | any prefix — all recoverable states |
| `unmount()` | `sync()` with clean flag | — |

Every intermediate crash state is: leaked space, an orphan inode, or a dangling directory entry pointing at kind=0 — all detectable, all repairable, none destructive.

### 7.4 `check()` — fsck-lite

Runs after an unclean mount (and any time). Flushes live state first, then walks the device:
1. From root, recursively validate directory chains: entry rec_len ≥ 12, 4-aligned, no straddle, `name_len ≥ 1`, kind ∈ {1,2}; target inode used and kind-agrees; each dir contains `.`/`..` pointing correctly; depth ≤ 512.
2. For every reachable file, validate extents (inside the data region, within total blocks, ordered, non-adjacent) and accumulate a reference bitmap of data blocks; regular-file invariant `(blocks−1)×4096 < size ≤ blocks×4096`.
3. Repairs (each counted in `Report`, all obeying the never-overwrite rule): dangling entries → holes; double references → strip the extent from the later-walked file; leaked bitmap bits (set, unreferenced, not metadata) → clear; referenced-but-clear bits (shouldn't exist; fix anyway) → set; leaked inodes → free with generation bump; rebuild link counts, free counters, SB.
4. Anything structural that cannot be repaired without risking data (extent pointing into the metadata region, root unreadable) → `Report.hard_corrupt = true`, minimal edits.

---

## 8. Test strategy

House style: `#[cfg(test)]` per module, byte-pinned codec tests (the `vfs-protocol/src/wire.rs` precedent), edge cases first-class.

1. **Per-module unit tests** (steps §9): fletcher32 known vectors; geometry arithmetic (tiny volumes, rounding, DeviceTooSmall); superblock byte pins + checksum corruption detection; bitmap run scan/hint/wrap/exhaustion/region cross-block; extent coalesce/spill/chain; inode codec pins; dir-block ops (hole split/absorb/tail, 255-byte name, block-full transitions, `'A'` vs `'a'` distinct lookups); path validation (all caps, NUL, `..`, trailing slash, case).
2. **Volume suite on `MemoryDevice`**: format→mount→unmount cycles; mount fallback to backup; magic/checksum rejection; unclean flag lifecycle; full file/dir operation matrix with error codes; trees under root (several sibling dirs, nesting ≥ 5 levels); grow across the 26-inline boundary into a spill chain; zero-fill gap reads; ENOSPC/NoInodeSpace on tiny volumes; stale-handle `BadHandle`; 65th open → `TooManyOpen`.
3. **Crash-injection harness** (`#[cfg(test)]` in `volume` tests): a `FaultDevice` (op recorder that panics-out after N device ops, re-created from a snapshot prefix) and a `VolatileDevice` (writes buffered, persisted only on `flush` — models a device write cache). For each abort prefix of scripted op sequences: remount (fallback where relevant), `check()`, assert `Report` clean-after-repair **and** all bytes committed before the last `sync()` are intact. This suite is the proof of §7.3 and the reason the crate exists as a pure library over a trait.
4. **Differential property test**: seeded random op streams (mkdir/create/open/write/append/read/remove/rmdir/lookup/readdir/sync) against a reference model (`BTreeMap<Vec<u8> path, Vec<u8>>` + dir set) on `MemoryDevice`; final states must match exactly; reruns at multiple seeds.
5. **Host-file backend**: suites 2–4 re-run with `FileDevice` on `/tmp/opencode/rxfs-*.img` (`cargo test -p rxfs --features std`), proving the image is just bytes a host tool can inspect (and, later, `xtask` can stamp for QEMU).

---

## 9. Steps (one commit each; every step green before the next)

| # | Deliverable | Gate |
|---|---|---|
| 1 | Crate scaffold: `fs/rxfs/Cargo.toml` (empty deps, `std` feature), workspace `members` + `default-members` add `fs/rxfs`; `lib.rs`, `spec.rs`, `error.rs`, `device.rs` (`BlockDevice`, `MemoryDevice` + tests) | `cargo test -p rxfs` |
| 2 | `checksum.rs` Fletcher-32 + vectors | `cargo test -p rxfs` |
| 3 | `superblock.rs` codec + `Geometry::plan` byte pins, checksum + fallback decision tests | `cargo test -p rxfs` |
| 4 | `bitmap.rs` bits/run-alloc/hint/wrap + region load/store + tests | `cargo test -p rxfs` |
| 5 | `extent.rs` `Extent`, inline+spill `ExtentList`, coalescing, spill-block serialize/deserialize + tests | `cargo test -p rxfs` |
| 6 | `inode.rs` 256-B codec, free representation, generation + tests | `cargo test -p rxfs` |
| 7 | `dir.rs` entry codec + block-stream find/add/delete/iterate + tests | `cargo test -p rxfs` |
| 8 | `path.rs` validation + component iterator + tests | `cargo test -p rxfs` |
| 9 | `cache.rs` direct-mapped write-back MetaCache + tests | `cargo test -p rxfs` |
| 10 | `volume.rs` format + mount + unmount + geometry wiring + volume suite (first half) | `cargo test -p rxfs` |
| 11 | `volume.rs` handle table + file ops (create/open/read/write/coalesce/spill/stat/close) + tests | `cargo test -p rxfs` |
| 12 | `volume.rs` path resolution + mkdir/rmdir/remove/readdir/lookup + full volume suite | `cargo test -p rxfs` |
| 13 | `sync()` commit ordering + bitmap/SB write-back + mount counter recompute + tests | `cargo test -p rxfs` |
| 14 | `check()` + `Report` + targeted-corruption tests (§8.2 list) | `cargo test -p rxfs` |
| 15 | crash-injection harness + differential property tests + `file.rs` `FileDevice`, suites re-run on both backends | `cargo test -p rxfs && cargo test -p rxfs --features std` |
| 16 | `AGENTS.md` sweep: structure tree gains `fs/rxfs` line; Kernel Subsystems/context bullet for RxFS (format summary + "no journal, leak-safe order, check()"); build note `cargo test -p rxfs` | greps: `fs/rxfs`, `rxfs` non-empty in `AGENTS.md` |

Steps 1–9 are pure/self-contained (no cross-module deps beyond spec/error); 10–13 build the engine in dependency order; 14–15 prove consistency; 16 is docs.

---

## 10. Explicitly out of scope (later themes)

- `fsd` daemon, `fs-protocol` treaty (or `vfs-protocol` growth: current wire caps names at 57 and has no paths — that boundary change belongs to that theme, not here), `vfsd` block-backed store swap, `xtask fsimg` host tool to stamp/pre-populate images for QEMU.
- The ATA `BlockDevice` adapter (lives with `fsd`).
- `rename()`, `truncate()` (D6); symlinks, hard links (beyond `.`/`..`), permission enforcement (mode stored, not checked), xattrs, sparse files/holes (D8), journaling (pad blocks 2..=7 are its reserved future home), COW/snapshots, allocation groups (volume scaling path), resize, read-only mount, 2-way cache/LRU, wall-clock mtime (no RosX clock source yet), Unicode case/normalization policy (names are bytes by D7).

## 11. Risks and accepted costs

1. **16 TiB volume / 16 TiB file cap** from u32 addressing (D2) — accepted.
2. **Direct-mapped cache thrashing** on pathological block strides — accepted v1 cost; recorded upgrade path.
3. **mtime is a txg stamp** — not wall time until RosX grows a clock; field layout reserves room for honest time later.
4. **Bitmap RAM = 32 KiB/GiB** — 128 KiB for a 4 GiB volume, fine for a user-space daemon; grouped bitmaps are the scaling note.
5. **Crash tests model abort + volatile devices**, not out-of-order completion; ATA PIO (today's only path) is synchronous, so the model is faithful; a barrier-aware device model is the note if async devices arrive.
6. **readdir index instability under concurrent mutation** — the single-`&mut` API plus the serializing daemon make it moot; documented.
7. **`cargo test` at the workspace root now builds `rxfs`** (default-members) — it is a plain host-buildable lib, no bare-metal entanglement; the bare-target proof is acceptance A3.

## 12. Acceptance (repo root, all green)

- **A1** `cargo test -p rxfs` and `cargo test -p rxfs --features std` — exit 0, suites per §8.
- **A2** `cargo test` (default members incl. `fs/rxfs`) — exit 0.
- **A3** Bare `no_std` proof: `cargo build -p rxfs --target arch/x86_64/rosx-user.json -Zbuild-std=core,alloc,compiler_builtins -Zbuild-std-features=compiler-builtins-mem` and `cargo build -p rxfs --target arch/x86_32/rosx-i686-user.json` + same `-Z` flags — exit 0 (forms quoted from `xtask/src/arch.rs` + `UNSTABLE`).
- **A4** `cargo xtask test --skip-integration` — exit 0 (the change is additive; only `Cargo.toml`/`Cargo.lock` outside `fs/`).
- **A5** Purity greps: `fs/rxfs/Cargo.toml` `[dependencies]` empty; `grep -rn "extern crate std\|use std" fs/rxfs/src | grep -v "cfg(test)\|feature = \"std\""` empty in lib paths (A3 is the stronger proof; A5 is the tripwire).
- **A6** Δ-new warnings = 0 vs baseline.
