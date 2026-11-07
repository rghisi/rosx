# Single-Disk Boot: drive0 Carries the Kernel, the Bootstrap ROM, and the Root Volume

> **Status:** Active — owner decisions D1-D5 ratified 2026-11-06; implementation started.
> Written 2026-11-06 against `init-decoupling` (PR #128, arc 4).
> **Repo law applies** (AGENTS.md): no comments in Rust, no commit-subject prefix,
> one commit per step, unit tests per module, kernel = mechanism / policy userspace.

---

## Goal

Arc 4 made the boot manifest data; the ROM is still the distribution. This arc moves
the distribution onto the disk, so one generated drive0 image carries everything and
the ROM shrinks to the bootstrap closure it can never shed:

1. **One image, drive0**: a partitioned FAT HDD image built by the toolchain — GRUB
   stages + `/boot/rosx` + `/boot/rom` + the full app roster + `INIT.LST`, `FSTAB`,
   and a new `ROOT` record, all as files (the era HDD recipe already does the boot half).
2. **Root volume from the device**: `vfsd` builds the VFS root engine from drive0's
   FAT volume (auto-detected, read-only this arc); an absent or foreign drive0 falls
   back to today's ROM-rxfs root, so nothing existing breaks.
3. **init resolves services through VFS**: `service` lines resolve ROM-record-first,
   VFS-second — with zero grammar change. Once the disk is root, the same manifest
   lines boot the same services straight off the disk.
4. **ROM becomes bootstrap**: init + atad + vfsd + shell + `fstab` + `init.lst` +
   `ROOT` ≈ 400 KiB (measured inputs below) instead of 1.12 MiB; the app library
   leaves with F3, and the era `-m 8` OOM (arc 4 evidence) cures itself by design.

**The permanent constraint** (why a bootstrap ROM never vanishes): the disk stack is
userspace — the kernel cannot read any filesystem until `atad` + `vfsd` run, so the
kernel's boot handoff must always carry their images out-of-band. This arc honors the
constraint rather than inventing a kernel-side disk driver.

**Port order**: x86_32 first — GRUB already delivers kernel + ROM module from drive0
files today (the era HDD is proof). Steps 1-2 are port-agnostic and benefit x86_64
immediately (its drive0 is raw, so it simply keeps the fallback root); full
boot-image-from-drive0 on x86_64 arrives with the Multiboot 2 entry (F1 below),
because GRUB cannot speak the `bootloader`-crate `BootInfo` protocol and Multiboot 1
cannot enter 64-bit.

## Verified source facts (2026-11-06, measured on `init-decoupling`)

- ROM today: **31 records** — 1,140,853 bytes (x86_32) / 1,200,621 (x86_64).
- Bootstrap set (x86_32 release ELFs): `init` 38,516 · `atad` 45,336 ·
  `vfsd` 247,268 · `shell` 48,780 bytes → a bootstrap ROM ≈ 400 KiB with records,
  `fstab`, `init.lst`, `ROOT`.
- `rxfat` has its **write side** (PR #125): `Engine::create/mkdir/rmdir` delegate to
  real implementations (`fs/fat/src/volume.rs:504-517`); guest FAT writes are proven
  by `fat_write.rs`. AGENTS' `fs/fat` line still says "read-only" — stale, corrected
  in the docs step.
- `BootSector::parse` demands a structurally coherent BPB (`fs/fat/src/boot.rs:54-79`:
  `reserved_sectors == 0` or `fat_count == 0` → `Corrupt`) — a zero-filled scratch
  disk or a raw kernel image can never false-positive as FAT; the fallback path
  answers honestly (the `NotFormatted` philosophy).
- The era HDD is already drive0-as-files: `build-legacy-hdd.sh` lays an `sfdisk`
  partition, stage1 in the MBR, `fat_stage1_5` in the gap, `/boot/rosx` + `/boot/rom`
  (script line 75); boots QEMU `-cpu 486 -m 8` to `rose>` (arc 4 rehearsal).
- `fat-image` today is a **superfloppy**: `FatDisk::new/add_file/geometry/as_bytes/
  write_to` (`fs/fat-image/src/lib.rs:93-164`) — no partition table, no subdirectories.
- Root is fixed by alias: `ActiveVolume = FileSystem<MemoryDevice>`,
  `VfsStore::new()` (`apps/vfsd/src/fs.rs:29-30,333`); mount auto-detect is one
  `BootSector::parse` (`fs.rs:289`); `MountedEngine` already implements `Engine` as
  an enum (`fs.rs:43`) — the exact precedent for an enum root.
- Routing law: a bare name hits the root engine at `/{name}` — apps at the FAT volume
  root answer the identical bare-name lookups the ROM volume answers today
  (`boot_and_shell` pins bare `ls` listing `tetris` and bare-name exec).
- Longest roster name is 7 chars (`waitset`/`threads`/`randomd`/`attach`) — the whole
  roster fits FAT 8.3 without LFNs.
- x86_64 boot protocol: `entry_point!` (bootloader-crate `BootInfo`), raw disk image
  with `set_ramdisk` (`arch/x86-runner/src/lib.rs:207-209`) — no Multiboot, no files.
  `cargo xtask image x86_32` already builds a GRUB 2 ISO with `multiboot` + `module`
  (lib.rs:195), and `iso_boot.rs` boots CD media headless (`QemuSession::spawn_cdrom`).
- init's handoff reader is already the service loader's shape: bounded
  `Vfs::connect`, size cap, truncated-image guard, ELF magic check
  (`apps/init/src/main.rs`).
- Era OOM (arc 4 evidence): `-m 8` (~5 MiB usable) panics inside `vfsd`'s whole-ROM
  `mount_rom` copy — on base too; shrinking the ROM is the cure, which is this arc.

## Design rules (locked)

- **I1 — drive0 layout**: partitioned HDD image, partition 1 FAT16 (FAT32 optional).
  Volume root carries the app roster + `INIT.LST` + `FSTAB` + `ROOT`; `/BOOT` carries
  `ROSX`, `ROM`, and the GRUB stages. Apps sit at the **volume root, not `/bin`** —
  bare-name routing must not change at all.
- **I2 — root-from-device is not a mount-table overlay**. A new ROM record `ROOT`
  (one line: `alias service drive`) names the root device. `vfsd` applies it before
  binding: attach, probe, auto-detect, build the root engine from the device; absent
  drive, foreign media, or any failure → keep today's ROM-rxfs root and log
  `[VFSD] root: {why}, rom fallback` (boot never fatal, the fstab skip philosophy).
  `FSTAB` grammar is untouched.
- **I3 — root engine = enum**: `BootVolume` (`RxfsMemory | Fat(FatFileSystem<...>)`)
  implements `Engine` — the `MountedEngine` precedent. `VfsStore` stays generic;
  only the `ActiveVolume` alias retargets. No wire change, no daemon-policy change.
- **I4 — the FAT root is read-only this arc**: create/write/mkdir/remove on the root
  volume answer `Unsupported` at the daemon edge (the initrd virtue; the write path
  is already proven for mounted data volumes by #125). Writable root is F2.
- **I5 — init resolution = ROM-record-first, VFS-second**, no grammar change; the
  lazily dialed shared reader is the handoff's (cap + truncation guard). No new
  pinned `[INIT]` strings — disk-root presence is proven by the `ROOT` test's marker
  file, not by log noise.
- **I6 — x86_64 is unchanged media-wise**: its raw drive0 falls back to the ROM root,
  so every x86_64 suite passes untouched from step 2 onward; the Multiboot 2 entry
  (F1) brings it into the drive0-only story without touching this arc's products.
- **I7 — kernel untouched**: the three-program roster (kstatsd, atad, init), the port
  grants, and the wire all stay exactly as arc 4 left them. Zero new syscalls.
- **I8 — the era gate**: the `-cpu 486 -m 8` rehearsal must reach `rose>` **with no
  allocation-failure panic** — this arc closes the arc 4 era-OOM evidence entry.

## Owner decisions (ratified 2026-11-06)

- **D1 — root format = FAT.** Ratified. The rxfs-partition alternative stays F5.
- **D2 — keep drive1 `/hdd`** as the optional writable data convention. Ratified,
  with the owner's explicit intent to remove it later — captured as **F7**; the hdd
  suites stay untouched this arc.
- **D3 — keep the `shell` ROM fallback** for this arc. Ratified; retiring it stays F4.
- **D4 — image builder = the era recipe first** (sfdisk/mformat/mmd/mcopy/dd + the
  pinned grub-shell container, all already trusted in `era-image`); a pure-Rust
  `fat-image` builder stays F3 — stage `setup` needs the container either way.
  Ratified.
- **D5 — the integration harness keeps booting `-kernel`/`-initrd`** (fast, stable);
  only `cargo xtask run x86_32` switches to the image, so development exercises the
  shipped medium. Ratified.

## Steps

1. **init resolver: ROM-first, VFS-second.** Lift the handoff reader into the
   `init` lib (`read_image(vfs, name)`), resolve each `service` name against the ROM
   records first, VFS second, lazily dialing one `Vfs` connection. Unit tests for the
   pure resolver decisions; no manifest or log-string changes.
   *Gate:* `cargo test -p init`; `boot_and_shell`, `init_handoff`, `ipc_random`,
   `ipc_bulk` green on both arches, byte-identical `[INIT]` story.
2. **`ROOT` record + `BootVolume` root.** `vfsd`: parse the ROM `ROOT` record (absent
   → today, byte-identical), else attach/probe/auto-detect the named device and build
   `BootVolume::Fat(...)` read-only; foreign/absent → ROM fallback + the `[VFSD] root:`
   line. `xtask` packs a default `ROOT` (`root0 ATA 0`) beside `fstab`.
   *Gate:* new `root_on_fat` integration test — `fat-image` drive0 carrying a marker
   file + the roster → bare `ls` shows the marker, bare-name exec runs a disk copy;
   plus the fallback assertion (zero-drive and raw-drive0 sessions keep booting).
   Full suites green on both arches.
3. **`cargo xtask disk-image x86_32`.** The single image: `sfdisk` partition + FAT16 +
   `/BOOT/ROSX` `/BOOT/ROM` + roster at volume root + `INIT.LST` `FSTAB` `ROOT`, GRUB
   Legacy stages via the pinned grub-shell (the `era-image` trust line); the FAT
   layout gets a tool-free round-trip check through `rxfat` at build time.
   *Gate:* image boots QEMU `-hda` to `rose>` with the `[INIT]` lines and the marker
   visible through VFS; `mtools mdir` third opinion.
4. **Dev media unified.** `cargo xtask run x86_32` boots the `disk-image` via `-hda`
   (`--direct` keeps the `-kernel`/`-initrd` path). Harness unchanged (D5).
   *Gate:* manual `-hda` boot to prompt + disk-root listing; the x86_32 suite still
   green through the `-kernel` harness path.
5. **Bootstrap ROM profile.** `cargo xtask rom x86_32 --bootstrap` (and the
   `disk-image` recipe defaulting to it): `init` `atad` `vfsd` `shell` + `fstab` +
   `init.lst` + `ROOT`; CI keeps the full ROM until this lands. Era rehearsal rerun:
   `-m 8` boots to `rose>` **without the OOM panic**.
   *Gate:* era evidence + `boot_and_shell`, `init_handoff`, `ipc_*` on the bootstrap
   media where the harness arch path allows.
6. **Docs + battery + PR.** AGENTS.md (root-on-disk story in both arch sections, the
   stale `fs/fat` read-only line corrected, `rom/` record census, `xtask` command
   list + `disk-image`), `plans/fat/README.md` arc-5 row, this plan's Evidence
   section. Full battery + `check-isa` + clippy.

## Follow-ups (documented, NOT this arc)

- **F1 — x86_64 Multiboot 2 entry**: MBI2 header + long-mode trampoline, `memmap` +
  `module` tag decode, a `boot_info` seam feeding both the bootloader-crate path and
  the MBI2 path, then a GRUB 2 (or grub2-ISO) recipe — after it, x86_64 joins the
  drive0-only boot with every step-1..5 artifact reused.
- **F2 — writable FAT root** (root mutators + `sync`, root-side directories).
- **F3 — tool-free `disk-image`**: `fat-image` grows directories + a partition table;
  the layout builds and round-trips without mtools (grub `setup` stays the pinned
  container step).
- **F4 — retire the `shell` ROM fallback** once disk-root boots are trusted.
- **F5 — the rxfs-root option**: partition-offset `BlockDevice` adapter + host rxfs
  formatter, if the FAT 8.3 name ceiling ever bites.
- **F6 — supervision** (inherited arc-4 F2, independent).
- **F7 — retire the drive1 `/hdd` convention** (owner intent behind D2): once the root
  volume is writable (F2), a second data drive loses its reason to exist — the default
  fstab entry, the harness's two-drive topology and the persisting-hdd suites go away.

## Evidence (to be filled during execution)

### Step 2 — `ROOT` record + `VolumeEngine` root (2026-11-07)

- New gate `root_on_fat` (x86_32): blank drive0 answers `[VFSD] root root0: no FAT
  volume, rom fallback` and keeps the ROM roster; a FAT drive0 (marker + `mem`
  `mkdir` `format` ELFs) answers `[VFSD] root root0 adopted from ATA 0` — bare
  `ls` shows `marker.bin` and no tetris (ROM ingestion skipped), `mem` executes
  from disk bytes, `mkdir data` answers `Vfs(Unsupported)`, `format root0`
  answers `Vfs(Busy)`.
- Unit additions: `fstab::parse_root` (6), `first_partition`/`partition_view` (5),
  root-policy store tests (6) — vfsd lib at 115 host tests. The x86_32 ROM now
  packs 32 records; x86_64 stays at 31 — its raw BiosBoot drive0 carries a real
  boot sector, so packing a `root` record there is deliberately deferred to F1
  (the probe may never gamble against `BootSector::parse` accepting boot-sector
  junk as a BPB).
- Full suites: x86_32 integration 0 failed; x86_64 integration 58 passed plus one
  `ipc_wake_latency` failure — a pre-existing timing flake on its console-order
  assertion (measured 2 failures / 7 recorded runs of it; the first x86_64 sweep
  was fully green; `plans/stack/deferred-reschedule-impl.md` documents flakiness
  in this family). The kernel, `ping`, `pong`, and `arch/` are byte-identical to
  `init-decoupling` on this branch — no wake-path code is in this arc.
- `cargo xtask test` (unit battery) clean; `check-isa x86_32`: kernel and 29
  user ELFs 486-clean; clippy: 34 vfsd-lib warnings vs 35 on base, none on
  touched lines.
