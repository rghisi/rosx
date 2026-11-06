# vfsd Integration: FAT Volumes in the Mount Table — Implementation Plan

> **Status:** Active. Decisions locked by owner 2026-11-06: **(1) auto-detect** the
> engine at mount, **(2) enum** for the heterogeneous mount table, **(3) `format`
> stays rxfs-only**. Baseline `fat-rxfat-engine` (stacked on open PR #122; retarget
> to `main` when it merges).

---

## Goal

Make a FAT volume a first-class citizen of the existing attach/mount flow so the
shell executes ELF files that live on a real FAT medium: `attach <dev-id> <service>
<device>` → `mount <dev-id> <dir>` → `rose> snake` runs the file off the disk, with
no rebuild in between. The shell launcher path needs **no change** — it already
streams ELFs by path through VFS (`LoadElf`); what changes is only what engine sits
behind the mount.

## The shape the change lands on (verified 2026-11-06 against this tree)

- `VfsStore<E: Engine, D: BlockDevice>` (`apps/vfsd/src/fs.rs:71`): root engine
  `E = ActiveVolume = rxfs::FileSystem<MemoryDevice>`, mount table
  `Vec<MountedVolume<D>>` where `MountedVolume { name, device, fs: FileSystem<D> }`
  is concretely rxfs (line 37); `OpenLocation::Mount { slot, handle: RxfsHandle }`
  is the only mount-side handle (line 45); every op dispatches through
  `volume_mut(slot)` (nine sites) in Engine style.
- `RemoteBlock` (`apps/vfsd/src/blk.rs`) is **4 KiB-block-addressed** — it maps
  rxfs's `BLOCK_BYTES` onto device sectors. FAT is 512-byte-sector addressed, so it
  cannot reuse this adapter; a second device type is the honest shape.
- rxfs validates its superblock by magic `FS_MAGIC` (8 bytes at offset 0 **of the
  superblock block**, `SUPERBLOCK_BLOCK = 1` → volume byte 4096;
  `fs/rxfs/src/superblock.rs:125`) — so the probe window must reach byte 4104,
  not just the first device sector; reading the first few device blocks answers
  "is rxfs" and "is FAT" (BPB at byte 0) without consuming the disk.
- `map_mount_error` (fs.rs:116) already maps `Corrupt`/`DeviceTooSmall` →
  `NotFormatted`; `VfsError::Unsupported` exists for geometry answers.

## Design (locked)

- **Engine enum (decision 2).** In vfsd (daemon-side dispatch, not `fs-core`):
  `enum MountedEngine<D, F> { Rxfs(rxfs::FileSystem<D>), Fat(rxfat::FileSystem<F>) }`
  implementing the `fs_core::Engine` treaty itself, with
  `enum MountedHandle { Rxfs(RxfsHandle), Fat(FatHandle) }` and
  `enum MountedMetadata { Rxfs(Stat), Fat(FatMetadata) }` (`MetadataView`
  delegates). The nine dispatch sites then keep calling Engine style; only
  `volume_mut`'s type and `OpenLocation::Mount`'s handle change.
- **Two device parameters.** `VfsStore<E: Engine, D: rxfs::BlockDevice,
  F: rxfat::device::BlockDevice>` — one type cannot serve both unit systems:
  rxfs counts 4 KiB blocks, rxfat counts 512-byte sectors, and `RemoteBlock`
  legitimately rejects foreign buffer sizes. `ActiveStore =
  VfsStore<ActiveVolume, RemoteBlock, FatSectors>`; host tests use
  `MemoryDevice`/a Vec-backed sector device unchanged.
- **`FatSectors` (`blk.rs`).** Sector-addressed read-only device over the
  `block-protocol` client: sector count = `block_count * block_size / 512`
  (requires `block_size ≥ 512` and `block_size % 512 == 0`, else the mount probe
  answers `Unsupported`); a sector read hits one device block and slices when the
  device block is larger than 512.
- **Auto-detect (decision 1), rxfs first.** At `mount_device`: connect, probe,
  read the probe window (`probe_head_blocks` device blocks — enough to cover the
  FAT BPB at byte 0 and the rxfs magic at byte 4096); `FS_MAGIC` at 4096 → rxfs
  engine (today's path, unchanged); else `BootSector::parse` valid with
  `FatType::Fat16 | Fat32` → FAT engine; else `NotFormatted`. FAT12-valid geometry answers `Unsupported` (the engine's
  honest capability answer, same as rxfs geometry rejects). Each engine is
  constructed exactly once — the probe never consumes the disk.
- **`format` stays rxfs-only (decision 3).** FAT volumes arrive pre-formatted
  from the host (`mformat`); no FAT write path in this arc.
- **Error mapping.** `map_fs_error` gains `FsError::Unsupported →
  VfsError::Unsupported`; FAT mount reuses `map_mount_error`.
- **ROM overlay precedence** needs no new code: mount routing already wins over
  root names for mount prefixes.

## Steps

1. `apps/vfsd/Cargo.toml` + `blk.rs`: `rxfat` dependency, `FatSectors` with
   pure geometry helpers unit-tested (`fat_sector_count`, device-block slicing).
2. `fs.rs`: `MountedEngine`/`MountedHandle`/`MountedMetadata` + `Engine` impl;
   third type parameter; dispatch-site retypes; `OpenLocation::Mount` carries
   `MountedHandle`.
3. `fs.rs`: `detect_format(head, sectors)` + `mount_device` branch; FAT12 →
   `Unsupported`; foreign → `NotFormatted`.
4. Tests: existing suites keep passing on the generic shape (`mount_fake` pushes
   the `Rxfs` variant); new: `detect_format` truth table, FAT fake-mount
   round-trip (list/stat/open/read/close through routing), write/mkdir/delete on a
   FAT mount → `Unsupported`, mount-table Busy/name rules treat FAT volumes the
   same, `sync` idempotence through the enum.
5. Gate: `cargo test -p vfsd` green, `cargo xtask test --skip-integration` green
   (no wire, kernel, or launcher change; QEMU suites exercise the rxfs path
   through the new enum).
6. Docs: AGENTS.md VFS architecture paragraph gains the FAT mount sentence; the
   `fs/fat` entry's "vfsd does not mount it yet" retires.

## Evidence (implemented 2026-11-06)

- `MountedEngine`'s `Rxfs` variant is boxed (`clippy::large_enum_variant`:
  rxfs `FileSystem` ≥1402 bytes vs rxfat's 92); one Box allocation per mount.
- The probe function is `probe_format(head, Option<device_sectors>) -> VolumeFormat`
  (names: `Rxfs`/`Fat`/`Unsupported`/`Foreign`); `head` is the first device block
  read once via the block client before either engine is constructed.
- Synthetic-FAT lesson (now encoded in `fat16_image` in the test module): a
  space-filled 32-byte slot must zero `0x14..0x16` — rxfat reads the cluster-high
  word there (FAT32 layout), so an un-zeroed slot decodes as `Corrupt`.
- Probe-offset bug caught by the QEMU suite (`attach_format_and_carve_directories_on_the_hard_disk_volume`
  mounted a just-formatted rxfs volume as `NotFormatted`): the first design read
  one device sector and checked `FS_MAGIC` at byte 0, but the magic lives at the
  superblock block (byte 4096). Fixed by the `probe_head_blocks` window; a
  regression test (`probe_ignores_rxfs_magic_misplaced_at_byte_zero`) pins the
  lesson. Host unit tests could not catch it because fake mounts bypass `mount_device`.

## Out of scope (next steps, recorded)

- QEMU integration test executing an ELF off a FAT medium: needs harness
  IDE-topology extension or xtask-built FAT media + CI mtools verification
  (`unverified`).
- FAT write path and `mkfs.fat` (needs the rxfat write arc).
- Partition carving (D4 in `README.md`): volumes are presented whole to the block
  service.
- Lifting one `BlockDevice` trait into `fs-core` (housekeeping; both daemon
  devices would then share one contract).
