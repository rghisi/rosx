# FAT Write: Make RosX a Good Citizen of Standard Filesystems

> **Status:** ACTIVE (arc 1 of 3). Branch `fat-write`, stacked on
> `fat-exec-test` (PR #124). Baseline: engine read path #122, mount #123,
> exec-from-FAT #124 all in the stack.
>
> **The sequence (owner decision 2026-11-06):** this arc makes FAT a
> two-way door (RosX writes files a standard system can read); arc 2
> (boot auto-attach/auto-mount) removes the per-boot manual ritual; arc 3
> is the parked `boot-packaging.md` (P1: kernel + ROM as files). Steps 2
> and 3 get their own plans when they start.

## Goal

A mounted FAT volume becomes writable through the full production path:
`cp`, `mkdir`, `rm`, `rmdir` from the shell flow through `vfsd` into
`rxfat`, mutate real FAT structures, and the disk pulled out of the
machine reads back correctly on a standard system (proven by mtools).
This is the bidirectional interchange that "manipulate files on a standard
system" means; today FAT is inbound-only (every `rxfat` mutator answers
`Unsupported`).

## Facts this plan rests on (verified 2026-11-06)

- `fs-core::Engine` carries the mutators already: `create`/`mkdir`/`rmdir`/
  `remove`/`write`/`sync`; `FsError` already names `AlreadyExists`,
  `NoSpace`, `InvalidName`, `FileTooLarge`, `DirectoryNotEmpty`.
- `MountedEngine::Fat(fs)` in `vfsd` already dispatches every mutator to
  the engine (`fs.rs:92-166`); the daemon side needs no semantic change —
  the moment `rxfat` stops answering `Unsupported`, the wire works.
- The block IPC is writable in production: `block-protocol::client::Block`
  has `write_blocks` and `flush`; `format ata0` already hammers them
  through `RemoteBlock`.
- `rxfat::device::BlockDevice` is read-only by construction
  (`block_count`/`read_block`); `FatSectors` in `vfsd/blk.rs` maps 512-byte
  engine sectors to device blocks (`sector_window`).
- Reader semantics that bind the writer: directory scan **stops at the
  first `0x00` slot** and skips `0xE5`/LFN/label slots; `decode_entry`
  rejects cluster-high != 0 on FAT16, dir-size != 0, and out-of-range
  clusters — every slot we write must satisfy the reader's strict decode.
- vfsd test `open_existing_with_create_is_already_exists` pins the treaty:
  `create` on an existing path answers `AlreadyExists` — FAT must match
  rxfs behavior, not POSIX O_TRUNC.
- The three mtools-enforced rules (mirrored FAT copies, media-tagged
  entry 0, FAT16 entry-1 high byte `0xFF`) are already encoded in the
  fat-image writer; its `slot_name`, chain-end, and FSINFO helpers are the
  reference recipe (no_std forbids a crate link — rxfat re-implements the
  few lines).
- `rxfat` lookup already descends nested directories (test
  `lookup_nested_directories_and_files`), so mkdir trees need no reader
  extension.

## Locked decisions

- **W1 — 8.3 only.** Create/mkdir fold names exactly like `fat-image`
  (uppercase, `[A-Z0-9_]`, stem 1..8, ext 0..3); anything else is
  `InvalidName`. No LFN creation. Readers still skip foreign LFN slots.
- **W2 — no `.`/`..` entries.** Our engine resolves by full traversal and
  the reader skips dot entries; real Linux/mtools tolerate their absence.
  Written directories contain only real entries. Foreign dot entries keep
  being skipped.
- **W3 — write-through, no cache.** Every mutation is a synchronous
  device write (sector RMW where needed). `sync()` remains `Ok(())`; the
  drive flush rides the existing atad `Flush` at the daemon edge. Crash
  discipline follows rxfs's leak-safe philosophy in FAT form: allocate +
  zero + link clusters **before** the slot references them; on remove,
  mark the slot deleted **before** freeing the chain. FAT has no journal;
  we do not invent one.
- **W4 — writes require a 512-byte device block.** ATA drives are 512
  everywhere in our fleet; `FatSectors::write_block` refuses adapted
  geometries honestly (`IoError::Device`) while reads keep slicing.
- **W5 — delete is `0xE5`**, standard: first byte marked, rest of the slot
  left intact; slot allocation prefers the first `0xE5` slot, else the
  terminator `0x00` slot (moving the terminator forward). FAT16 root
  exhaustion answers `NoSpace`; chain-rooted dirs (FAT32 root, subdirs)
  extend by allocating one zeroed cluster.
- **W6 — open handles pin the slot.** `OpenEntry` records the parent
  directory location and the slot's sector+offset at open/create time;
  every write re-validates the slot still holds the same name bytes,
  else `BadHandle`.
- **W7 — no truncate, no rename, no in-guest FAT format.** The Engine
  treaty has no truncate or rename; `format` stays rxfs-only by the
  existing decision — FAT media arrives from `cargo xtask fat-disk`.
- **W8 — grow with zero-fill.** Writing past EOF extends the chain and
  zero-fills any gap (no sparse files in FAT); writes cap at the 32-bit
  FAT size field (`FileTooLarge`).

## Milestones

1. **Device write capability.** `rxfat::device::BlockDevice` gains
   `write_block(&mut self, block: u64, buf: &[u8]) -> Result<(), IoError>`;
   every existing reader type (rxfat fixtures, fat-image differential
   adapters) becomes writable (their backing is already `Vec<u8>`).
   `fat.rs` gains entry addressing for writes (both copies) and an FSINFO
   free-count update. Nothing calls it yet.
2. **Volume mutation core.** Slot-level writer in `dir.rs`: 8.3 fold
   (W1), slot scan (W5), slot install/erase, strict-decode-clean bytes
   (cluster-hi lesson included). Cluster-level writer in `fat.rs`: free
   scan from cluster 2, chain extend, chain free, zero-on-alloc, both
   copies, FSINFO upkeep. Unit tests drive these directly through the
   fixture device.
3. **Engine mutators.** `create` (resolve parent, reject existing per
   treaty, install slot), `write` (extend/truncate-free grow W8, sector
   RMW, slot refresh, W6 guard), `remove` / `rmdir` (empty-check via the
   reader's own decode, W3 ordering), `mkdir` (zeroed cluster, dir attrs).
   `Engine for FileSystem` now routes all six instead of `Unsupported`;
   the old `mutators_answer_unsupported` test inverts into real
   expectations.
4. **vfsd write adapter.** `FatSectors::write_block` via
   `block::client::Block::write_blocks` behind the W4 gate. No other
   daemon change — the enum already routes.
5. **Differential proof, both directions.** rxfat-built mutations on an
   mformat-built volume read back by `mdir`/`mcopy` (the read arc's image,
   now round-tripped by the *guest* writer); deletes invisible to mtools;
   a `fat-image` volume grown by rxfat still passes the mtools gate.
   skip-with-note when mtools is absent.
6. **QEMU integration + CI.** A `fat_write` suite (both arches): boot
   with a `fat-image`-built FAT16 disk, mount it, `cp mem fat/mem2` +
   `mkdir fat/sub` + `cp echo fat/sub/e` + `rm`, assert via `ls`, reboot
   the session and assert persistence, then **host-side** verify the
   guest-written bytes with `mcopy` byte-for-byte (mtools-gated). CI picks
   it up with no tool installs, as in #124.

## What this arc deliberately does not do

- **LFN creation** (W1), **rename**, **truncate**, **chmod/ownership on
  FAT** (no such fields; vfsd's ownership model stays daemon-side and is
  not persisted to FAT).
- **FAT12**, **non-512 device writes** (W4), **fsck/journal** — FAT is
  FAT; we keep it leak-safe, not magical.
- **Auto-mount** (arc 2), **boot packaging P1** (arc 3,
  `boot-packaging.md`).

## Residual risks and mitigations

- **Mid-update power loss** leaves a leaked-cluster or stale-slot volume
  (any FAT does). Mitigations: W3 ordering makes leaks the only failure
  mode (never double-allocation), and the read arc's `check`-style
  tolerance plus mtools remain diagnostic options.
- **Slot identity under concurrent mutation** (two open handles, rm
  between writes): the W6 name-byte revalidation turns a would-be
  corruption into `BadHandle`.
- **Semantics drift vs rxfs**: the same vfsd daemon tests exercise both
  engines via `MountedEngine`; the integration suite runs identical shell
  scripts against rxfs (existing `vfs_hdd_flow`) and FAT (new `fat_write`),
  so divergence shows up as a red test, not a surprise.

## Evidence (2026-11-06)

- `rxfat` unit tests: 120 green (from 93 at arc start) — mirrored entry
  addressing, free-cluster scan, FSINFO upkeep, the allocator family
  (allocate/append/free with zero-on-alloc and `NoSpace` without partial
  writes), slot mechanics (8.3 fold accept/reject matrix, E5-over-append
  preference, positioned lookup, erase), and eleven engine scenarios:
  round trip, collision/bad-name/missing-parent matrix, cluster-straddling
  growth with zero-filled gap, `FileTooLarge` before any touch, remount
  persistence, cluster reclamation on remove, W6 slot guard (`BadHandle`
  after remove), mkdir with children, rmdir rules, full-FAT16-root
  `NoSpace`, and FAT32 root chain extension keeping FSINFO truthful.
- mtools write differentials (skip-with-note when absent):
  `files_written_by_rxfat_read_back_through_mtools_{fat16,fat32}` — creates,
  a nested mkdir+child, a delete and an in-place patch all visible to
  `mdir`, extracted content byte-equal via `mcopy` (mdir prints padded
  8+3 columns, so assertions match the name column, not dotted names).
  `mtools_reads_a_{fat16,fat32}_volume_grown_by_rxfat` (fat-image crate):
  volumes built by `fat-image`, mutated by `rxfat`, still pass the mtools
  trio; the deleted roster file answers `mdir` with exit 1.
- `vfsd`: 81 daemon tests green; `fat_mount_refuses_every_mutation_as_
  unsupported` inverted into `fat_mount_mutations_flow_through_daemon_
  routing` — create, write, mkdir, overwrite-existing, remove, readdir and
  reopen all flow through the mount table with zero daemon policy change.
- QEMU integration `fat_write` (both arches): guest `cp mem fat/mem2`,
  `mkdir fat/sub`, `cp echo fat/sub/e`, `rm fat/mem`; reboot; the second
  boot sees every effect; then `rm fat/sub/e` + `rmdir fat/sub`; host-side
  `mcopy` extracts MEM2 byte-identical to the source ELF and `mdir` reports
  MEM and SUB gone. 45s on x86_64, 15s on x86_32 (`-cpu 486`).
- Harness gotcha: the integration harness reuses prebuilt user ELFs —
  the first run exercised a stale ROM (`Vfs(Unsupported)`); a
  `cargo xtask build [arch]` before `cargo test -p tests-integration` is
  mandatory after touching any app or engine crate.
- `cargo xtask check-isa x86_32`: kernel + 28 user ELFs 486-clean.
- Durability closure found mid-review: rxfs's `BlockDevice` trait carries
  `flush` (the contract behind its `sync_makes_writes_survive_power_loss`
  test), so the FAT path gained it too: `FileSystem::sync` →
  `FatSectors::flush` → block-protocol `Flush` → atad FLUSH CACHE, reached
  per-request by vfsd's existing sync-after-mutation discipline.
  `sync_flushes_the_device` pins the hop.
- Full-suite runs after the flush commit: x86_64 integration 26 suites,
  exit 0, zero failures (`fat_write` 44s); x86_32 (`-cpu 486`) integration
  26 suites, exit 0, zero failures (`fat_write` 15s); workspace unit suites
  all ok. `check-isa x86_32` re-run on the rebuilt ELFs: kernel + 28 user
  ELFs 486-clean.
