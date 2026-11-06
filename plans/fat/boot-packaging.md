# Boot Packaging: Kernel + ROM as Files on FAT Media — Parked Plan

> **Status:** **PARKED** by owner decision 2026-11-05 (the rxfat engine ships first;
> this arc may resume afterwards or be superseded). Fully specified so it resumes
> without re-investigation. Baseline `main` @ `6e184bc`.

---

## Goal

Retire `include_bytes!` from both arch ports: the kernel binary and the app roster
(`rom.img`) live as ordinary files on FAT boot media, and the bootloader hands the
ROM to the kernel in memory:

| Port | Medium | ROM delivery |
|---|---|---|
| x86_64 | the `bootloader`-crate MBR+FAT image (already FAT) | `BiosBoot::set_ramdisk(rom.img)` → `BootInfo.ramdisk_addr/ramdisk_len` |
| x86_32 dev (QEMU `-kernel`) | none (loader bypass) | `-initrd rom.img` → Multiboot v1 module 0 |
| x86_32 era HDD | GRUB Legacy 0.97-67 FAT16 (`build-legacy-hdd.sh`) | `module /boot/rom` in `menu.lst` |
| x86_32 ISOs | El Torito / GRUB 2 mkrescue | same `module` line |

All names stay 8.3-safe (`rosx`, `rom`) — GRUB Legacy reads no LFN.

## Verified source facts (2026-11-05, vendored sources + pinned deb)

- `bootloader_api 0.11.17` `BootInfo` carries `ramdisk_addr: Optional<u64>` +
  `ramdisk_len: u64` (`src/info.rs:56-58`); `BiosBoot::set_ramdisk` exists
  (`bootloader-0.11.17/src/bios/mod.rs`); stage 2 loads the `ramdisk` file right
  after the kernel (`bootloader-x86_64-bios-stage-2-0.11.17/src/main.rs:105-108`).
- x86_64 `kernel_main(boot_info)` already reads `BootInfo`
  (`arch/x86_64/src/main.rs:62`); memory blocks come from
  `build_memory_blocks` filtering `Usable` regions (line 116).
- x86_32 `kernel_main(multiboot_magic, multiboot_info)` (`arch/x86_32/src/main.rs:42`)
  decodes memory via `x86_generic::multiboot::usable_memory`; Multiboot v1 module
  list = `flags` bit 3, `mods_count` u32 @20, `mods_addr` u32 @24 — each module is
  `{module_start, module_end, string, reserved}`. **(Corrected 2026-11-06: this
  line originally said @16/@20 — those are `cmdline` and `mods_count`; the first
  QEMU boot caught the slip and `multiboot.rs` pins the spec offsets in a test.)**
- Boot rosters are `include_bytes!` statics (`arch/x86_64/src/main.rs:74-107`,
  `arch/x86_32/src/main.rs:55-84`): ROM image + 8 daemon ELFs + shell.
- `system::rom` exposes `RomImage::parse` + `files()` (and `encode`)
  (`system/src/rom.rs:18-83`) — the kernel links `system` and can extract ELFs by
  name itself.
- `xtask` packs `USER_APPS` (27 names) into `rom.img`
  (`xtask/src/main.rs:11,92-112`); **`shell` is not in the ROM** — packing it in is
  the one deliberate roster change (the shell becomes visible on the ROM volume;
  the kernel boots the same bytes).
- QEMU multiboot loader takes `-initrd <file>`; GRUB Legacy `menu.lst` takes a
  `module /boot/rom` line; GRUB 2 `multiboot` config likewise.
- The era media scripts already stage `/boot/rosx` on FAT16 / ISO9660
  (`build-legacy-hdd.sh:72`, `build-legacy-iso.sh:27`).

## Memory-safety rule (why carve, not copy)

A `Vec::from_slice` copy of the ROM would race the global allocator: the allocator's
first chunk may *contain* the ROM region, and its bookkeeping can write into the
source before/while the copy reads it. Deterministic fix on both ports: **carve the
ROM's physical range out of `MemoryBlocks` before `Kernel::bootstrap`**, keeping the
ROM a zero-copy `&'static [u8]` — `kernel/src/rom.rs` (a `Once<&'static [u8]>`,
`kernel/src/rom.rs:3-10`) needs no change at all. This is independent of whether
each loader's memory map already excludes the region.

## Steps

1. **`arch/x86-generic`: Multiboot v1 module decoding.** Parse `flags` bit 3 +
   `mods_count`/`mods_addr`, return module ranges (address + name); host unit tests
   over synthetic info blocks; nothing wired yet.
2. **`xtask`: pack `shell` into the ROM** (roster 27 → 28).
3. **x86_32 cutover.** `kernel_main`: module 0 → carve from `MemoryBlocks` →
   `rom::init`; boot roster (incl. shell) extracted by name via `system::rom`;
   delete the ten `include_bytes!`; runner passes `-initrd`. Verify:
   `cargo xtask run x86_32` → `rose>`, `ls` shows the ROM files.
4. **x86_64 cutover.** Runner gains a rom-image argument, calls `set_ramdisk`;
   `kernel_main` reads `ramdisk_addr/len` + carves it in `build_memory_blocks`;
   same name-lookup boot; includes deleted. Verify: `cargo xtask run`.
5. **Media scripts.** `grub_cfg_contents()` (`arch/x86-runner/src/lib.rs:154`) and
   both `build-legacy-*.sh` recipes stage the ROM + `module` line. Verify all media
   in QEMU (`-cpu 486` on x86_32).
6. **Integration harness.** `QemuSession` spawns pass the ROM (initrd on x86_32);
   full suites green on both arches.
7. **Docs.** AGENTS.md packaging section; retire "the ROM carries the app ELFs (not
   the shell)".

## Follow-up (after both arcs): one common HDD

Unify both ports on a single FAT16 HDD image by moving x86_64 to GRUB 2 +
Multiboot2 (retiring `bootloader_api` BootInfo for MB2 info tags; the x86_32 port's
v1 parsing is the shape reference). Until then FAT is the only claim: both media are
FAT disks built by different tools.
