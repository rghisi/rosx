# rxfat — Read-Only FAT16/FAT32 Engine — Implementation Plan

> **Status:** Active, step-by-step. Written 2026-11-05 against `main` @ `6e184bc`.
> **Depends on:** decisions D1–D5 in `README.md`.
> **Deliverable:** a new workspace crate `fs/fat` (package `rxfat`) implementing the
> `fs-core` `Engine` treaty as a read-only FAT16/FAT32 engine over a 512-byte-block
> device, with rxfs-grade host tests including differential tests against real
> `mformat`-built images. No kernel, vfsd, or app changes in this plan.
> **Gate every milestone:** `cargo test -p rxfat` green; arc end additionally runs
> `cargo xtask test`.

---

## Scope and non-goals

**In (v1):** read-only FAT16 and FAT32 volumes; 512-byte sectors; power-of-two
1..=128 sectors/cluster; 8.3 names; nested directories; fragmented cluster chains;
`lookup/open/read/readdir/stat/close/sync`; mutators → `Unsupported`.

**Out (honest `Unsupported` or contract):** FAT12; non-512-byte sectors; LFN
reconstruction (records skipped); writes; volume carving from whole-disk images
(D4); FSInfo use (read-only engine never needs the free counters).

## Format facts the engine codes against (Microsoft FAT spec, 1996–2000)

- Boot sector: `bytes_per_sector` u16 @0x0B; `sectors_per_cluster` u8 @0x0D;
  `reserved_sectors` u16 @0x0E; `fat_count` u8 @0x10; `root_entry_count` u16 @0x11;
  `total_sectors_16` u16 @0x13; `fat_size_16` u16 @0x16; `hidden_sectors` u32 @0x1C;
  `total_sectors_32` u32 @0x20; FAT32 adds `fat_size_32` u32 @0x24 and
  `root_cluster` u32 @0x2C. File-system-type string at 0x36 (FAT12/16) or 0x5A
  (FAT32); `0x55 0xAA` at 0x1FE.
- FAT type is decided **only by cluster count**: < 4085 → FAT12, < 65525 → FAT16,
  else FAT32. The type string is informational.
- Cluster numbers are valid from 2. Special FAT entries: 0 free; FAT16 `0xFFF7`
  bad, `>= 0xFFF8` end-of-chain; FAT32 `0x0FFFFFF7` bad, `>= 0x0FFFFFF8`
  end-of-chain, top 2 bits of every FAT32 entry reserved (mask on read).
- Derived: `root_dir_sectors = ceil(root_entry_count * 32 / bytes_per_sector)`;
  `first_data_sector = reserved + fat_count * fat_sectors + root_dir_sectors`;
  `cluster_count = (total − first_data_sector) / sectors_per_cluster`;
  `sector_of_cluster(c) = first_data_sector + (c − 2) * sectors_per_cluster`.
- Directory entry (32 bytes): name 8 @0, ext 3 @8, attrs u8 @0x0B (0x01 read-only,
  0x08 volume label, 0x0F LFN, 0x10 directory), first-cluster lo u16 @0x1A /
  hi u16 @0x14 (hi only meaningful on FAT32), size u32 @0x1C. First byte 0x00 ends
  the region, 0xE5 is a deleted slot, 0x05 is a literal 0xE5 name start.

## Milestones

### M1 — Boot sector decoder (`fs/fat/src/boot.rs`) — no device needed yet

- Workspace: add `"fs/fat"` to `members` **and** `default-members`; new
  `fs/fat/Cargo.toml` mirroring ramfs (package `rxfat`, deps: `fs-core` only);
  `src/lib.rs` with `#![cfg_attr(not(test), no_std)]` + `extern crate alloc;`.
- `FatType { Fat12, Fat16, Fat32 }` decided by cluster count.
- `RootLocation { Sectors { first: u64, entries: u32 }, Cluster(u32) }` — FAT12/16
  root lives in a fixed sector range, FAT32 root is an ordinary cluster chain.
- `BootSector::parse(volume_sector_zero: &[u8], device_sectors: u64)
   -> Result<BootSector, FsError>` with derived fields: sector size, sectors per
  cluster, fat count, fat sectors, root location, total sectors, first data sector,
  cluster count, FAT type. Validation rules:
  - `0x55AA` missing **or** no `"FAT"`-prefixed type string at 0x36 **and** 0x5A →
    `Unsupported` ("not a FAT volume" — mount auto-detection rides this answer).
  - `bytes_per_sector`: power of two in 256..=4096, else `Corrupt`; exactly 512 or
    `Unsupported` (D3 device contract).
  - `sectors_per_cluster`: power of two 1..=128, else `Unsupported`.
  - `fat_count` 0 or > 2 → `Corrupt`.
  - FAT32 layout (`fat_size_16 == 0 && root_entry_count == 0`): `fat_size_32`
    nonzero and `root_cluster >= 2`, else `Corrupt`. FAT12/16 layout: nonzero
    `fat_size_16`, nonzero `root_entry_count` that is a multiple of
    `bytes_per_sector / 32`, else `Corrupt`.
  - total sectors (16-bit, falling back to 32-bit) nonzero or `Corrupt`; volume
    geometry (total < first_data_sector) `Corrupt`; device smaller than the volume
    declares → `DeviceTooSmall`.
- `first_sector_of_cluster(cluster: u32) -> Option<u64>` — `None` outside 2..=range.
- **Tests:** hand-built FAT16 (cluster_count mid-FAT16) and FAT32 geometries with
  exact derived values; type thresholds at 4084/4085 and 65524/65525; every rejection
  rule above; `first_sector_of_cluster` boundaries.

### M2 — Device contract + FAT chain walking (`device.rs`, `fat.rs`)

- `pub trait BlockDevice { fn block_count(&self) -> u64;
   fn read_block(&mut self, block: u64, buf: &mut [u8]) -> Result<(), IoError>; }`
  (read-only shape, D5). The test device lives in `#[cfg(test)]` (no test-only
  production surface).
- `FatTable::next(&mut self, cluster) -> Result<ClusterNext, FsError>` where
  `ClusterNext { End, Next(u32) }`: entry sector/offset by width (u16 FAT16, u32
  masked `0x0FFFFFFF` on FAT32); free entry (cluster referenced as data), bad entry,
  and reserved clusters 0/1 → `Corrupt`; `FatType::Fat12` → `Unsupported`.
- `chain(device, start_cluster) -> Vec<u32>`: iterate with a cap of
  `cluster_count` steps — exceeding it means a loop → `Corrupt`.
- **Tests:** synthetic FAT sectors (hand-written bytes) for both widths; chain of
  length 1/N; EOC recognition; bad/free/reserved/loop → `Corrupt`.

### M3 — Directory entries (`dir.rs`)

- Entry decoder: 8.3 fold (trim spaces, join `base.ext`, ASCII-lowercase, 0x05 →
  0xE5), attrs → `is_dir`, `first_cluster` (lo | hi<<16), `size`.
- Region walker: root-as-sectors (FAT16) and root/directory-as-chain (FAT32 and all
  subdirectories) share one iterator over the device; stops at first byte 0x00;
  skips deleted (0xE5), volume labels (0x08), and LFN records (0x0F) — LFN is an
  alias per D3, the short entry carries the same file.
- `name_matches(stored, requested)`: ASCII case-insensitive equality (FAT stores
  upper-case; callers pass lower-case names).
- A directory entry claiming `is_dir` with size ≠ 0, or a file's cluster ≥
  valid range → `Corrupt` at decode.
- **Tests:** hand-written directory sectors covering every skip rule, 0x05 names,
  case folding, dotless-ext-empty entries, end-of-region, cross-sector continuation.

### M4 — File read path

- `read_region(device, boot, start_cluster, offset, buf) -> usize`: walk to the
  cluster containing `offset`, serve `sectors_per_cluster`-aligned slices,
  zero-fill past EOF (mirror ramfs), truncate to `buf.len()` and remaining size.
- **Tests:** small file inside one cluster; file spanning clusters; read straddling
  a cluster boundary; offset past EOF → 0; partial zero-fill of the final cluster's
  tail.

### M5 — Engine implementation (`volume.rs`, re-export in `lib.rs`)

- `FileSystem<D: BlockDevice> { boot: BootSector, device: D, opens: Vec<Option<OpenEntry>>, generation }`;
  `FileSystem::new(device)` parses the boot sector from block 0 (maps M1 errors) and
  rejects `FatType::Fat12` as `Unsupported`.
- `Handle = FatHandle { slot: u32, generation: u32 }` (Copy, Debug, PartialEq);
  open-table snapshot of the directory entry makes reads generation-stable; stale
  or double close → `BadHandle`.
- Path model mirrors ramfs `path::parse` (absolute, no empty segments, 4096-byte
  paths, 255-byte names, depth ≤ 512; the module is rxfat's own copy — no ramfs dep).
- Ops: `lookup` walks segments from the root scanning directories (case-insensitive);
  `open` on a directory → `IsADirectory`; `readdir` lists entries (no synthetic
  `.`/`..`, matching ramfs); `stat` from snapshot or path-walk per treaty;
  `read` via M4; `close` frees the slot; `sync` → `Ok(())` (read-only volume).
  `create/mkdir/rmdir/write/remove` → `Unsupported`.
- Device `IoError` maps to `FsError::Io` everywhere.
- **Tests:** a whole in-memory FAT16 volume assembled byte-wise in `#[cfg(test)]`
  (boot sector, FATs, root dir, subdirs, fragmented files) driven through the full
  treaty: happy paths, NotFound/NotADirectory/IsADirectory/BadHandle, mutator
  answers, `sync` idempotence, two volumes on one engine type.

### M6 — Differential tests vs real mtools images

- Host-gated test (`Command`-probe, skips with a printed note when mtools is
  absent): `mformat` a superfloppy FAT16 and a FAT32 image (sizes chosen so mtools
  picks each type), `mcopy` in nested files from tempdir fixtures, `mdir`/host bytes
  as ground truth → engine `readdir` names/kinds and full file bytes match,
  including an 8.3-safe multi-cluster file and an empty file.
- Corrupt-image edge battery over a real image: flip boot sector bytes → the M1
  rejection answers, proving mount auto-detect honesty on real media.

## Step order and commits

One commit per milestone, subjects without prefix, working tree green before each:

1. M1 `Decode FAT boot sectors into a validated volume geometry in rxfat`
2. M2 `Walk FAT16 and FAT32 cluster chains in rxfat`
3. M3 `Decode FAT directory entries with the LFN-as-alias skip rule in rxfat`
4. M4 `Serve cluster-relative FAT file reads with EOF zero-fill in rxfat`
5. M5 `Give rxfat the read-only Engine treaty: lookup, open, readdir, stat`
6. M6 `Prove rxfat against real mformat-built FAT16 and FAT32 images`

Then the arc stops at the vfsd integration plan (`vfsd-integration.md`), which is a
separate PR with open questions for the owner.
