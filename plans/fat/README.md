# FAT Arc — Packaging a Regular HDD for RosX

> **Status:** Active. Written 2026-11-05 against `main` @ `6e184bc`.
> **Owner decisions locked 2026-11-05:** start with the userspace FAT engine (not the
> boot surgery); hand-roll it like rxfs (zero third-party deps); the boot/ramdisk
> packaging plan is parked, not dropped — picked up as arc 3 on 2026-11-06.
> **Repo law applies** (AGENTS.md): no comments in Rust, commit subjects carry no
> prefix, one commit per step, unit tests per module.

---

## Why this arc exists

The goal is to move packaging from the current scheme — app ELFs and `rom.img` baked
into the kernel binary with `include_bytes!` (`arch/x86_64/src/main.rs:74-107`,
`arch/x86_32/src/main.rs:55-84`) — to a regular HDD carrying the kernel and all apps
as ordinary files on a FAT filesystem, and to read app files from disk at runtime
(loose, host-editable files, not just a build-time pack).

A bootloader survey was run on 2026-11-05; every fact below was verified against the
vendored crate source or the pinned GRUB deb:

- **x86_64** boots via rust-osdev `bootloader` 0.11.17 `BiosBoot`
  (`arch/x86-runner/src/lib.rs:167`). Its stage 2 reads **FAT only** — Fat12/Fat16/Fat32
  (`bootloader-x86_64-bios-stage-2-0.11.17/src/fat.rs`, `main.rs:78` — the kernel comes
  from a root-dir file named `kernel-x86_64`). The host tool already writes the medium
  as MBR + stage-2 slice (type 0x20) + a **FAT32** partition
  (`bootloader-0.11.17/src/lib.rs:111`, `src/mbr.rs`, `src/fat.rs`).
- **x86_32 era HDD** boots via GRUB Legacy 0.97-67 `stage1` + `fat_stage1_5` on FAT16
  (`arch/x86-86box/scripts/build-legacy-hdd.sh`) — proven on the emulated 486SX. The
  pinned deb's stage1_5 set (verified with `dpkg -c`): `e2fs`, `fat`, `jfs`, `minix`,
  `reiserfs`, `xfs`. GRUB Legacy has **no LFN support** — anything it must open stays
  in 8.3.
- **x86_32 ISOs**: `stage2_eltorito` (ISO9660, `build-legacy-iso.sh`) and GRUB 2
  `grub-mkrescue` (also ISO9660 on that medium; installed GRUB 2 `i386-pc` reads
  FAT12/16/32 and ext2/3/4 via modules).
- QEMU `-kernel` (the x86_32 dev path) bypasses boot filesystems entirely.

**Conclusion: FAT16 is the packaging target** — the only family every live boot path
reads, already rehearsed end-to-end on the 486SX medium. ext2 is blocked by the
x86_64 bootloader (FAT-only stage 2) and is fragile with GRUB Legacy's 1997-era
`e2fs_stage1_5` vs modern `mkfs` feature sets.

## What each plan covers

| Plan | Scope | Status |
|---|---|---|
| `rxfat-read-engine.md` | New `fs/fat` crate (`rxfat`): hand-rolled read-only FAT16/FAT32 `Engine` over a block device, rxfs-grade tests | **Done** (PR #122) |
| `vfsd-integration.md` | Mount auto-detect (FAT vs rxfs by boot sector), FAT volumes in the mount table, shell exec-by-name from a FAT volume | **Done** (PR #123) |
| `fat-exec-test.md` | Rust FAT image builder (`fat-image`, no external tools), QEMU test executing an ELF off a superfloppy FAT disk, `xtask fat-disk` roster packing | **Done** (PR #124) |
| `fat-write.md` | FAT write engine in `rxfat` (create/write/mkdir/remove), vfsd write adapter, guest-writes-read-by-mtools round trip; arc 1 of 3 toward a fully usable OS (arc 2: boot auto-mount, arc 3: P1 boot packaging) | **Done** (PR #125) |
| `boot-automount.md` | ROM fstab + vfsd boot pass: attach+mount the convention drive before the shell exists, so the persistent disk is there at the prompt; arc 2 of 3 | **Done** (PR #126) |
| `boot-packaging.md` | Kernel + `rom.img` on FAT media delivered via ramdisk/Multiboot module, retiring `include_bytes!`; arc 3 of 3 | **Active** |

## Locked decisions

- **D1 — Hand-rolled engine.** Zero third-party deps, links only `fs-core`, matching
  rxfs/ramfs lineage (owner choice over wrapping the registry `fatfs` crate).
- **D2 — Package name `rxfat`.** `fatfs` is taken: `x86-runner` transitively depends on
  registry `fatfs 0.3.6` via `bootloader`, so a workspace member with that name collides
  at resolution. `rxfat` follows the rxfs precedent: RosX's engine for the format.
- **D3 — v1 capability set:** read-only FAT16 + FAT32, 512-byte sectors, 8.3 names.
  FAT12, non-512 sector sizes, and all mutators answer `Unsupported` — the honest treaty
  answer. LFN records are skipped on read (they are only aliases: mtools always writes
  the short-name entry alongside, so 8.3-only already interops with host-built media).
- **D4 — Volume-relative addressing.** Device block 0 *is* volume sector 0 (superfloppy
  shape, what `mformat -i img@@offset` produces). A whole-disk image whose volume starts
  at a partition offset needs a carved device upstream; the engine ignores `hidden_sectors`
  and says so through the contract, not through silent math.
- **D5 — Device trait placement.** `BlockDevice` currently lives in rxfs
  (`fs/rxfs/src/device.rs:7`), not in `fs-core`. `rxfat` defines its own **read-only**
  same-shaped trait now; a separate housekeeping step can lift one trait into `fs-core`
  (it is already expressed purely in `fs-core::IoError`, so the lift is pure
  contract-relocation). Deliberately *not* mixed into this arc.
- **D6 — Boot packaging stays parked.** The ramdisk/Multiboot-module design is fully
  specified in `boot-packaging.md` with every source fact cited, so it can resume
  without re-investigation.

## Verified-baseline facts (this session, against 6e184bc)

- `Engine` treaty: twelve operations with associated `Handle` + `Metadata` types
  (`fs/core/src/engine.rs:22-48`). `FsError` has `Unsupported` (line 28) plus
  `NotFound/IsADirectory/NotADirectory/BadHandle/Corrupt/DeviceTooSmall/...`
  (`fs/core/src/error.rs:8-29`).
- `ramfs` shape to mirror: `fs/ramfs/Cargo.toml` (dependency: `fs-core` only),
  `#![cfg_attr(not(test), no_std)]` + `extern crate alloc;`, `path::parse` model
  (`fs/ramfs/src/path.rs`: absolute, no empty segments, `MAX_NAME_BYTES` 255,
  `MAX_PATH_BYTES` 4096, `MAX_DEPTH` 512).
- Workspace: `fs/*` members are listed explicitly in root `Cargo.toml` (`members` from
  line 2, `default-members` from line 46); `fs/fat` must be added to both.
- mtools 4.0.49 present on the dev host (`/usr/bin/mformat|mcopy|mdir`) for the
  differential test step.
