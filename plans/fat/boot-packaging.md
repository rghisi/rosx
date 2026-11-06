# Boot Packaging: Kernel + ROM as Files on FAT Media

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
  **(Corrected 2026-11-06 by QEMU measurement: the shipped stages hand
  `ramdisk_addr` as a VIRTUAL address already mapped into the kernel page
  tables — a dedicated window at 2^41 whose first word is the `ROSX` magic —
  so the kernel uses it verbatim and must NOT add `phys_offset`. The physical
  home lands inside the loader's reserved `[kernel .. last_used_addr]` gap,
  which the `Usable`-filter in `build_memory_blocks` excludes structurally —
  no x86_64 carve is needed, unlike x86_32 where the module address is
  physical and sits inside a usable mmap region.)**
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
   `kernel_main` reads `ramdisk_addr/len` as a ready virtual slice (see the
   corrected fact above — no carve needed); same name-lookup boot; includes
   deleted. Verify: `cargo xtask run`.
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

## Evidence log (2026-11-06)

- **Decode + carve + lookup:** `x86_generic::multiboot::boot_modules` (spec offsets
  pinned by test — the first QEMU boot caught the @16/@20 slip, see the correction
  above); `MemoryBlocks::carve` — 8 units; `kernel::rom::file_in` — 4 units;
  x86-generic 32 units.
- **ROM roster:** pack_rom now carries 28 ELFs + fstab (29 entries); kernel
  binaries shrank 3.5 MB → 196 KB (x86_32) and 1.8 MB → 205 KB (x86_64).
- **x86_32:** boots its whole roster from Multiboot module 0 (QEMU `-initrd`):
  boot_and_shell 4/4. GRUB 2 ISO (module line) boots to `rose>` (iso_boot, 10.5 s).
  Era media (GRUB Legacy 0.97-67, `module /boot/rom`): FAT16 HDD and El Torito ISO
  both boot to `rose>` in QEMU `-cpu 486 -m 8`.
- **x86_64:** `set_ramdisk` → `BootInfo.ramdisk_addr` read VERBATIM as virtual
  (measured: dedicated window at 0x20000000000, first word `ROSX`; physical home
  inside the loader's reserved gap — no carve needed, `Usable` filter covers it).
  boot_and_shell 4/4, ipc_ping / ipc_bulk / vfs_hdd_flow green.
- **Known era quirk (pre-existing, NOT from this arc):** one
  `alloc.rs handle_alloc_error` panic per era boot at -m 8..16 regardless of
  packaging — reproduced identically on boot-automount media rebuilt side by
  side; the 8 MB era gate still reaches `rose>` on both. The m=12 window is a
  marginal-heap lottery on both builds. Small-RAM budget work is its own story.
- **Full battery:** `cargo xtask test` exit 0 (1274 unit tests); x86_64 and
  x86_32 integration suites both exit 0 (64 passed / 0 failed each side).
  `check-isa x86_32`: kernel and 28 user ELFs 486-clean — after one guard
  fix: the boot's first `rep stos` (the module decoder's array zeroing)
  exposed that the audit table listed only suffixed `stos*` spellings while
  objdump prints `stos` bare for `f3 ab`; STOS is 8086-era, so the audit now
  allow-lists the string-family bases (`stos/movs/cmps/scas/lods`, suffixed
  forms covered by the existing suffix rule).
