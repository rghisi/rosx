# FAT Exec Test: Rust Image Tooling and ELF-by-Name off Disk — Implementation Plan

> **Status:** Proposed. Refined with the owner 2026-11-06: the B+C idea merges into
> **one Rust image builder driven by xtask** — building the tooling means dropping
> the external `mtools`/`sfdisk` dependency from the media path. **Partitions and
> the rxfs-root layout are explicitly deferred** (owner call: keep this arc
> simple): all media here is superfloppy (whole disk = one FAT volume), carving
> (D4) stays parked. Depends on the `vfsd-integration.md` arc (mount auto-detect,
> engine enum — PR #123).

---

## Goal

Prove the user-visible promise of the FAT arc end to end: an ELF file that
arrives as a **loose file on a FAT volume**, attached and mounted through the
normal flow, **executed by name** from the shell — with the media itself built
by RosX's own Rust tooling, no external utilities. Deliverables: a FAT image
*writer* crate, a QEMU integration test executing an ELF off FAT on both x86
ports, and an `xtask` command that packs the built user roster onto a FAT disk
for developers, 86Box, and hardware.

## Facts this plan rests on (verified 2026-11-06)

- The mount side is shipped: `mount` auto-detects FAT by boot sector and serves
  it through the `MountedEngine::Fat` variant (`apps/vfsd/src/fs.rs`, arc
  `vfsd-integration.md`). FAT mounts refuse **all mutations** — images must be
  complete at build time, which is exactly what a builder provides.
- `tests-integration` already drives a data disk through the full
  attach/mount/exec-capable flow (`QemuSession::spawn_with_hdd_disk`,
  `tests-integration/src/qemu.rs:42`; the `vfs_hdd_flow` suite passes on both
  arches), and already owns a tool-gated boot test with graceful skip
  (`tests/iso_boot.rs` — x86_32 + host GRUB tooling or skip with a note).
- The synthetic-FAT recipe is proven twice: the rxfat unit fixtures
  (`fs/fat/src/fat.rs` fixtures module) and the vfsd fake mount
  (`fat16_image` in `apps/vfsd/src/fs.rs` tests), including the slot lesson:
  a space-filled 32-byte slot must zero `0x14..0x16` (cluster-high per FAT32
  layout) or rxfat decodes `Corrupt`.
- Geometry constants are mtools-verified from the read arc's differential suite
  (`fs/fat/tests/host_images.rs`: real `mformat` FAT16/FAT32 images read by
  rxfat, skipped-with-note when mtools is absent — the skip pattern to reuse).
- The two live boot media are **partitioned** (BiosBoot: `create_mbr_disk` puts
  FAT32 at an LBA offset, `bootloader-0.11.17/src/mbr.rs:46-58`; era-image:
  `sfdisk` + `mformat -i "$img@@$PART_OFFSET"`,
  `arch/x86-86box/scripts/build-legacy-hdd.sh:63-67`) — mounting a *boot* disk
  is therefore carving work (D4) and out of this arc; a superfloppy data disk
  needs nothing beyond what already shipped.
- Workspace members are an explicit list in the root `Cargo.toml` — a new crate
  is registered there, not by glob.

## Milestones

1. **`fat-image` crate** (`fs/fat-image`, package `fat-image`; std — it is host
   tooling, so no `no_std` constraint; registered in `members` +
   `default-members`). Not `fatfs`: the D2 name-collision rule still binds.
   - Builder API (shape): create an empty superfloppy FAT16 or FAT32 volume of
     a requested capacity, `add_file(name, &[u8])` (8.3 names; the guest folds
     them to lowercase, so `SNAKE.ELF` runs as `snake` per shell convention),
     multi-cluster chains for files bigger than one cluster, `write_to(path)`.
   - Sizing obeys the validated thresholds: FAT16 needs ≥ 4086 data clusters,
     FAT32 ≥ 65525 with the cluster-root layout; FAT sectors sized to cover
     `cluster_count + 2`; slot bytes follow the recipe incl. the zeroed
     cluster-high word.
   - Primary gate: **round-trip through the real `rxfat` reader** (dev-depend
     `rxfat` + `fs-core`): boot parse, root listing, multi-cluster read byte for
     byte, FAT32 variant. Optional third opinion (skip-with-note pattern):
     `mdir`/`mcopy` differential when mtools is present — the dependency inverts
     from the read arc: mtools becomes advisory, never required.
2. **QEMU exec test** (`tests-integration/tests/fat_exec.rs`, **both arches, no
   external tools**): build a small superfloppy FAT16 in a tempdir carrying one
   real app ELF (choice at implementation: an app with a deterministic console
   marker — `pi` is the candidate), attach as the HDD disk, then
   `attach <id> <service> <dev>` → `mkdir` mount point → `mount` (auto-detect
   must pick `Fat` — a misprobe surfaces as `NotFormatted`) → `ls` shows the
   folded name → execute by name → assert the marker on the console. This
   exercises the whole production read path for a ~100 KiB+ multi-cluster file:
   `FatSectors` per-sector IPC requests, chain walking, the launcher's
   size-bounded `LoadElf` read. No rxfs `format` step appears anywhere in the
   script — the single disk is FAT, period.
3. **`cargo xtask fat-disk [arch] [--fat32] [--size MiB] [--out path]`**: packs
   the built user roster for the arch (the `target/rosx-*-user/release/` ELFs)
   onto one superfloppy FAT disk, prints the file table it wrote. No external
   tools (reuses none — `fat-image` does the layout); distinct from
   `era-image`, which remains the GRUB-blob path.
4. **CI gate**: confirm `ci.yml` picks up the new unit suite (workspace test
   run) and the `fat_exec` binary (auto-discovered `cargo test -p
   tests-integration`), with no new tool install; the mtools-differential tests
   keep their skip-with-note behavior.

## Test design (milestone 2, detail)

- Flow assertions mirror `vfs_hdd_flow`'s expect-output scripting so a probe or
  dispatch regression fails with a readable console transcript, not a timeout
  alone.
- Negative case in the same test or a sibling: `format` on the mounted FAT
  device refuses (`Busy`) — the mount-table guard must treat FAT mounts exactly
  like rxfs ones on the real IPC path, not only in daemon unit tests.
- Both arches ride the existing `Arch::current()` harness dispatch; no new
  QEMU topology (`spawn_with_hdd_disk` is already the single-disk shape this
  test wants).

## What this arc deliberately does not do (all recorded, with homes)

- **Partitions / carving (D4).** The prerequisite for mounting *boot* media:
  the BiosBoot x86_64 disk and the era-image GRUB HDD are MBR-partitioned
  (cited above). Carving will be a windowed device adapter for **both** engines
  plus an MBR-aware probe, designed engine-neutral so it is done once.
- **The single boot+data disk test** (GRUB loads the kernel from P1, guest
  mounts that same P1 and execs apps) — needs D4, then reuses everything here.
- **rxfs root on a second partition / root-from-disk** — flips the per-boot
  RAM-root + ROM-overlay model and needs owner decisions (persistence, write
  path, mount-at-boot semantics). Vision, not scope.
- **`boot-packaging.md`** (kernel + `rom.img` as Multiboot modules) — stays
  parked; the two-partition layout the owner sketched is this arc unparked, on
  partitioned media, after D4.
- FAT **write** path in the guest, LFN long names, multi-partition images.

## Residual risks and mitigations

- **Self-reference**: our writer validated by our own reader could agree on a
  wrong layout. Mitigations: (1) layout constants and edge behaviors come from
  the mtools-differential evidence of the read arc, not from re-derivation;
  (2) the optional `mdir` third opinion when mtools is present; (3) milestone 2
  executes through the real QEMU boot path, and the image is directly
  inspectable with any host tool during development.
- **8.3 name folding**: app names longer than 8 chars truncate in the short
  name; the roster check in `fat-disk` must refuse or warn on names that collide
  after folding (`snake.elf` vs `snakelong.elf` → both `snake`-ish prefixes).
