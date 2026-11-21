# x86_32 Carrier Swap — GRUB out, Custom Chain In on a Unified Boot Disk

> **Status:** Executing. Written 2026-11-21 on branch `replace-grub-x8632` at HEAD `54034e0`;
> M1 executed 2026-11-21 (see its execution record; §5 verification law met per milestone).
> **Goal:** retire GRUB and QEMU `-kernel` as the x86_32 boot path. The x86_32 kernel boots
> through the custom chain of `arch/x86-bootloader/` (stage1 MBR → stage2 → ELF32 load →
> `BootInfo` handoff) from the **unified 64 MiB FAT boot disk** — the same image the boot plan
> already mounts as `/` — so `kernel.elf` on disk stops being a forward-looking artifact and
> becomes the real boot artifact.
> **Method:** every code fact below is cited to a path (and lines) re-verified against this
> tree. Facts the plan cannot prove at authoring time are marked **unverified** and routed to
> the milestone that must prove them.
> Repo law applies throughout: `system::boot_info` is **frozen** — this plan consumes it and
> changes nothing in it; no comments outside the frozen baseline (assembly excepted); 486 floor
> unchanged; commits bare imperative; every milestone lands green through `cargo xtask check`,
> and the series ends with `cargo xtask verify` as merge evidence.

---

## 0. Goal & Non-Goals

### Goal
- `cargo xtask build x86_32` writes `target/rosx-i486-boot-disk.img` as a **bootable unified
  disk**: MBR carrying stage1 boot code, the stage2 blob (`ROS2` image) at LBA 8, one bootable
  FAT16 slice at LBA 2048 (moved from 63) carrying every user ELF at the FAT root plus the
  chain kernel `KERNEL.ELF` — at the root, exactly where stage2 already looks (F15) — and
  `/boot/rom.img`.
- The x86_32 kernel grows a **dual-magic entry**: `EAX == BOOT_INFO_MAGIC` (`'ROSX'`) walks the
  `BootInfo` path (normalized `MemoryRegion[]` → `MemoryBlocks`); `EAX == 0x2BADB002` keeps
  today's Multiboot path byte-for-byte.
- `cargo xtask run x86_32`, `cargo xtask test` (x86_32) and `cargo xtask verify` boot through
  the chain: the bootable master at `ide,index=0` **is** the medium; `-kernel` is retired for
  x86_32 and `BootMedium::MultibootKernel` is deleted.
- The integration harness's full-apps masters are stamped bootable the same way, so every
  x86_32 session disk-boots through the chain to the disk shell.
- `cargo xtask image x86_32` (the GRUB-mkrescue ISO today) becomes the unified disk itself;
  `create_grub_image`, `GrubTooling` and `BootMedium::CdImage` are deleted, and
  grub-mkrescue/xorriso leave the host-tool story for everything except era media.
- Docs follow each milestone; `plans/bootloader/00-overview.md` §9 stubs *Real-kernel
  integration* (x86_32 half) and *Carrier swap* (x86_32 half) close here.

### Non-Goals
- **No x86_64 change of any kind.** Its `bootloader`-crate carrier, secondary-bus invariant,
  and the x86_64 real-kernel stub of §9 stay open on their own track.
- **No rom.img-as-initrd** (decision D5 of the bootloader plan stays open): the carrier ships
  no `INITRD.BIN` (the proven `handoff32-no-initrd` path), and the kernel keeps its
  `include_bytes!` ROM/init coupling. That is its own successor plan.
- **No deletion of the Multiboot header or `x86_generic::multiboot` in this series.** The
  dual-magic kernel keeps the fallback; the retirement lands as a later sweep recorded in
  `docs/todo/`.
- **No GRUB Legacy era-media change.** `cargo xtask era-image` and the `arch/x86-86box`
  scripts stay as-is: they are hardware-evidence tooling. Era verification of the custom chain
  (86Box `b32`, real-drive INT 13 margins, the recorded Award-BIOS thunk geometry defect) stays
  open bring-up work, gated by the AGENTS.md hardware-evidence rules — nothing here goes green
  on hardware.
- No change to `system::boot_info`, the handoff register contract, or any wire format.

---

## 1. Verified baseline facts

Cited against `54034e0`.

- **F1** The x86_32 kernel is Multiboot v1: header and trampoline in
  `arch/x86_32/src/boot.S` (`_start` installs its own flat GDT and 64 KiB BSS stack, then
  pushes `EBX`/`EAX` and calls `kernel_main`, lines 37–64); `kernel_main` rejects anything but
  `0x2BADB002` and parses the Multiboot mmap (`arch/x86_32/src/main.rs:40–51`,
  `arch/x86-generic/src/multiboot.rs`), trimmed below `kernel_end` by `trim_to_safe_memory`
  (`main.rs:85–103`).
- **F2** The stage2-Flat32 handoff is entry-compatible with that trampoline: protected mode,
  paging off, `CLI`, A20 on, `EAX = BOOT_INFO_MAGIC`, `EBX` = physical `BootInfo` pointer,
  16-aligned `ESP` (`arch/x86-bootloader/stage2/src/elf32.rs:23–26`,
  `plans/bootloader/00-boot-abi.md` §3). `boot.S` replaces GDT and stack regardless of the
  loader's, so **no kernel assembly changes are needed** — only the Rust-side magic branch.
- **F3** The kernel ELF is exactly the shape stage2 already loads: class-1 `EM_386`, fixed
  paddr, `. = 1M` (`arch/x86_32/linker.ld:4`) — the same placement `fake-kernel-32` uses
  (`arch/x86-bootloader/fake-kernel-32/linker.ld`) — zero paddr and class-2 images are refused
  (`arch/x86-bootloader/common/src/elf32.rs:111–131`). The planner reads the phdr table from a
  4 KiB header window (`stage2/src/elf32.rs:23` `HEADER_WINDOW`, `:29–35`), so the real
  kernel's phdr position is a **measured** requirement: **unverified**, M1 spike.
- **F4** The chain media layout is fixed and slice-agnostic: stage1 loads the `ROS2` blob from
  fixed LBA 8 into `0x7E00` with a 160 KiB budget (`arch/x86-bootloader/stage1/boot.s:23–24`,
  `xtask/src/boot_image.rs:16–28`), captures the BIOS boot drive in `DL` (`boot.s:38`), and
  stage2 opens the **bootable slice off the MBR table**, not a baked-in offset
  (`stage2/src/blk.rs:54` `boot_slice`). A 64 MiB volume with its slice at LBA 2048 is
  therefore a data choice, not a chain change.
- **F5** stage2 looks for `KERNEL.ELF` at the volume root (`stage2/src/elf32.rs:23`) and
  tolerates an absent `/INITRD.BIN` (`stage2/src/fat.rs:14–33`, checkpoint
  `handoff32-no-initrd`). Path lookup is case-insensitive 8.3 (`fs/fat/src/dir.rs:47`); the
  unified disk stages the real kernel at the root under that exact name (C5, F15), and the
  8.3 discipline both packers enforce (`xtask/src/boot_disk.rs:56–67`) keeps every name
  legal — `KERNEL.ELF` collides with no `USER_APPS` stem.
- **F6** The FAT boot disk today is 64 MiB, MBR stamped by `rxpart::mbr::encode_mbr` (generic
  code, partition table, signature — **no bootable stage1 code**) with the FAT16 slice at
  **LBA 63** (`xtask/src/boot_disk.rs:15,69–86`). The stage2 blob region (LBA 8–~328) lives
  *before* 2048 and does not collide with a slice moved there; `mformat` only writes the
  volume from the partition offset, so blob-then-format order is safe.
- **F7** The harness masters duplicate that layout with the same LBA-63 constant
  (`tests-integration/src/fat_disk.rs:10`), and every x86_32 session goes through
  `x86_runner::prepare_run_medium_in` + `qemu_command` (`tests-integration/src/qemu.rs:62–168`);
  `spawn_with_master_disk` attaches the **caller's** image file directly at index 0
  (`qemu.rs:73–88`) — the `disk_boot` suite boots the real `target/rosx-i486-boot-disk.img`
  through it, so harness-side stamping must happen on a **copy**, never in place.
- **F8** Medium selection is one match arm: `run_medium(X86_32) → MultibootKernel`
  (`arch/x86-runner/src/lib.rs:62–67`) and `qemu_command` renders it as `-kernel` (`:206–208`);
  the GRUB software surface is `GrubTooling`/`create_grub_image` (`:92–171`), the `CdImage`
  arm (`:209–213`), and `create_flashable_image`'s x86_32 branch
  (`xtask/src/main.rs:207–224`). The x86_64 branch of the same function keeps using
  `create_bios_disk_image` (`:180–190`) and is untouched.
- **F9** No-cargo-inside-cargo law: `prepare_run_medium_in` executes **inside** `cargo test`
  (integration harness), so the chain image builder must never invoke `cargo` (the target-dir
  lock would deadlock the session). stage1/stage2 binaries must be **pre-staged artifacts**,
  exactly like `ROSX_KERNEL_ELF` pre-stages the kernel.
- **F10** The stage2-32 `rxfat` engine is a one-sector-per-FAT-copy LRU cache with a cluster
  memo (`fs/fat/src/fat.rs:39,103–152`): FAT-table caching does **not** scale with volume
  size. But per-lookup directory reads allocate `Vec`s the bump heap never reclaims (F15) —
  volume size is free, path depth is not.
- **F11** The verdict law and its timeout class exist (`plans/bootloader/00-overview.md` §7;
  `stage2-corrupt-header` is a checkpoint that passes **on** timeout), and the checkpoint
  verifiers already read stage banners off the debug-port stream — the plumbing a gated
  real-kernel checkpoint needs.
- **F12** CI: the `bootloader` job runs `boot-test` + `check-isa bootloader` and does **not**
  build any kernel (`.github/workflows/ci.yml:162–186`) — it must stay that way, because
  building the x86_32 kernel requires apps + ROM (`arch/x86_32/build.rs` asserts both). The
  era job's GRUB-Legacy stage cache (`ci.yml:97`) is era tooling and stays.
- **F13** `cargo xtask verify` (added today, `xtask/src/main.rs:352–377`) is the merge-evidence
  bar: gate + build + integration on both arches + boot-test. Its x86_32 legs are exactly what
  this swap re-routes; after the swap they prove the real kernel boots through the chain in
  every session.
- **F14** Boot-plan invariant: both ports boot the same plan mounting `ata0d0p0`
  (`docs/architecture/boot-media-and-harness.md:5`). The unified disk **keeps that topology
  identical** — the kernel merely starts from the MBR of the very volume the plan mounts; the
  secondary-bus carrier invariant is x86_64's and is unaffected.
- **F15** (measured at M1, 2026-11-21) stage2's bump allocator has a **no-op `dealloc`**
  (`arch/x86-bootloader/stage2/src/heap.rs:41`), so every transient `rxfat` directory-read
  `Vec` leaks for the whole boot: peaks are **39,296 / 65,536 bytes** on the Flat32 disk-boot
  flow and **55,552 / 65,536** on stage-64 — an extra path component (`/BOOT/KERNEL.ELF`)
  costs ~10 KiB and OOMs the 64-bit chain with `[ERR] heap`. The window cannot grow: the heap
  already ends exactly at the `0x30000` linker `ASSERT` bound. Recorded as
  `docs/todo/stage2-bump-heap-never-frees.md`. Consequence: **the chain kernel stays at the
  volume root** (C5, revised) and future stage2 lookups must budget for F15.

---

## 2. Decisions

| # | Question | Resolution |
|---|----------|------------|
| C1 | Carrier topology | **Unified single disk** (chosen over a secondary-bus carrier): the 64 MiB FAT boot disk carries stage1 MBR code + stage2 blob + the kernel, and is simultaneously the boot medium and the `ata0d0p0` root volume. It is the "forward-looking artifacts" endgame the boot-media docs already point at, and it is what a real 486 IDE disk looks like. Slice moves LBA 63 → 2048 (F6). |
| C2 | Multiboot disposition | **Dual-magic now, retire later.** The kernel keeps the `.multiboot` header and the Multiboot memory path as a documented fallback; deletion is a follow-on sweep recorded in `docs/todo/` once the chain has soaked in. |
| C3 | Initrd | **None yet.** No `INITRD.BIN` on the carrier (`handoff32-no-initrd` path); `include_bytes!` coupling and D5 stay open as their own successor. |
| C4 | Stage-binary staging | `cargo xtask build x86_32` stages `target/boot32/{stage1.bin,stage2-32.bin}` (the same binaries `boot-image`/`boot-test` consume, built once). `cargo xtask test` and `verify` build them before handing off to `cargo test`; the by-hand `ROSX_KERNEL_ELF=…` flow runs `cargo xtask boot-artifacts` (new subcommand) first, and `x86-runner`'s image builder fails loudly naming that command when the staged bins are missing. Never cargo from inside the harness (F9). |
| C5 | Kernel path on the volume | **Revised at M1 (F15): the kernel lives at the FAT root as `KERNEL.ELF`** — stage2's existing constant, so **no bootloader change at all**; the unified disk stages the real kernel there (apps and the kernel share the root, `KERNEL` collends with no `USER_APPS` name). The `/BOOT/KERNEL.ELF` variant this plan first specified was implemented and **measured to OOM the stage-64 chain** (F15), so it is rejected; x86_32 therefore drops the `/boot/kernel.elf` copy (the root file is the mechanism), while `/boot/rom.img` staging and the x86_64 `/boot/kernel.elf` forward-looking artifact stay untouched. The frozen ABI is untouched either way. |
| C6 | Boot order determinism | The harness carves persistent HDD images carrying an MBR signature (`tests-integration/src/qemu.rs:45–56`), so the boot disk attaches with an explicit `bootindex=1` — mirroring the x86_64 carrier's explicitness rather than trusting SeaBIOS drive order. |
| C7 | Flashable image | `cargo xtask image x86_32` **is** the unified disk (single artifact). Without apps on the volume the boot plan already falls through to `exec rom:shell`, the same posture today's kernel-only GRUB ISO gives. |

---

## 3. Milestones

Gate rule as always: milestone N+1 starts only when N's proof is green and committed. Each
milestone carries its own docs commits; M5 closes the series.

### M1 — Shared chain packer + bootable unified disk (user space only)
- Lift the chain media packing (MBR+stage1 stamping, `ROS2` blob write, FAT16 volume, mtools
  copies) from `xtask/src/boot_image.rs` into `arch/x86-runner` (the host crate both xtask and
  `tests-integration` already depend on), leaving `boot_image.rs` calling it — the 24-checkpoint
  `boot-test` corpus must stay green off the shared code, unchanged.
- `create_boot_disk` gains the boot stamp: stage1 code + one bootable FAT16 0x06 entry at LBA
  2048 (via the `rxpart` layout `boot_image.rs` already uses), blob write at LBA 8 before
  `mformat`, the kernel staged at the FAT root as `KERNEL.ELF` (C5 revised, F15),
  `/boot/rom.img` and apps per the existing conventions (F6).
- Stage `target/boot32/{stage1.bin,stage2-32.bin}` as `cargo xtask build x86_32` artifacts
  (C4).
- **Spike proof (F3 unverified → measured):** boot `target/rosx-i486-boot-disk.img` on
  `qemu-system-i386 -cpu 486`. Before M2 the honest verdict is stage banners through
  `[ELF]`/`[INITRD]`-absent plus the real kernel's `bad Multiboot magic: 0x524f5358` panic on
  the debug stream — that panic **is** the proof that stage2 loaded the real ELF and jumped to
  its entry. Record measured kernel-ELF load time in the milestone notes.
- Gate: `cargo xtask check`, `cargo xtask boot-test`, manual QEMU boot above.

**M1 execution record (2026-11-21):**
- Lift landed in `arch/x86-runner/src/chain.rs` (ROS2 blob, stage1-MBR splice + guards,
  sparse/write, mtools wrappers) with 9 unit tests; `boot_image.rs`, `boot_disk.rs` and
  `boot_test.rs` call it; `x86-runner` gained a workspace `rxpart` dependency (arch-rules
  updated); `boot-test` 24/24 green off the shared code, unchanged.
- `cargo xtask build x86_32` now stages `target/boot32/{stage1.bin,stage2-32.bin}` and writes
  the bootable unified disk (stage1 MBR, `ROS2` blob at LBA 8, slice at LBA 2048, kernel at
  volume root as `KERNEL.ELF`, apps + `/boot/rom.img` as before); new
  `cargo xtask boot-artifacts x86_32` stages the chain binaries for the by-hand flow (C4).
- **Spike measured, `-cpu 486` -m 128M:** the chain loaded the real kernel ELF —
  `[ELF] arch=X86_32 entry=0x00101000 phys=0x00100000..0x001c0000` (a 768 KiB image; the
  phdr table fits the 4 KiB header window, **F3 closed**) — absent-initrd path clean, and
  handed off `[STAGE2_32] A20=OK THUNK=OK` in **437 ms POST → handoff** (boot-time risk
  closed).
- The jump **lands**: the QEMU monitor shows the CPU executing with the kernel's own GDT
  installed (`GDT=0x00126000`, CS 0x08/DS 0x10, CR0.PE=1 PG=0). The bad-magic panic does not
  print: `kprintln!` unwraps `Err` while `DEFAULT_OUTPUT` is unwired (`bootstrap()` wires it
  after the magic check), so the panic handler recurses into silent stack ruin. **M2 must
  wire the console (VGA init + default output) as the first act of `kernel_main`, before the
  magic branch** — recorded in M2 below.

### M2 — Kernel dual-magic entry (x86_32)
- **Console first:** `kernel_main`'s first act is `vga init` + wiring the default kernel output
  (M1 found the panic-before-`bootstrap` silent-recursion hole); the unknown-magic failure must
  be loud on both console streams. This needs `setup_default_output` reachable from the arch
  crate (`kernel/src/platform/output.rs`, `pub(crate)` → `pub`; both branches then pass the
  same `MULTIPLEXED_OUTPUT` static to `bootstrap`, so the `Once` stays idempotent and x86_64
  behavior is untouched).
- `kernel_main` branches on the magic; new `BootInfo` → `MemoryBlocks` consumer next to
  `trim_to_safe_memory`: walk `memory_map_ptr[..len]` (stride-checked against
  `memory_map_entry_size`), keep `Usable` regions only, cap at `MemoryBlocks` capacity
  (`BootInfo` allows 64 map entries vs the kernel's 32-region array — the consumer drops
  overflow loudly), reuse the `kernel_end` trim; `DisplayKind::VgaText` at `0xB8000` matches
  the existing `VgaOutput` (assert, do not re-derive). Unit tests over synthetic maps
  (low/lowmem-hole/high/multi-entry/overflow/zero-entry) in the module under test.
- Multiboot path untouched; every current suite rides `-kernel` and stays green unchanged.
- New checkpoint `real-kernel-handoff32` in `boot-test`, **gated on the release kernel's
  presence** (skipped, loudly, when absent — F12 keeps the `bootloader` CI job kernel-free):
  pack the real kernel, expect `[ELF]` + the kernel's own early `[KERNEL] Initializing` banner
  on the debug stream, verdict = timeout-class pass (F11).
- Measure the `-cpu 486` INT-13 window throughput end-to-end; if the kernel load adds more
  than ~2 s to a checkpoint boot, raise stage2's `PLACEMENT_WINDOW` (one-line lever) and
  re-measure.
- Gate: `cargo xtask check`, full x86_32 integration suite (still `-kernel`), `boot-test`
  including the new row.

### M3 — The disk is the medium
- `run_medium(X86_32)` returns the unified disk image; `BootMedium::MultibootKernel` is
  deleted; `qemu_command` attaches the boot disk at `ide,index=0` with `bootindex=1` (C6);
  `prepare_medium`/`launch` in xtask follow (F8). Audit the `cargo run`/`run.sh` path so the
  interactive flow gets the same treatment or says clearly why not.
- Harness: `fat_disk` masters get the boot stamp (staged bins in, F9); `spawn_with_master_disk`
  copies the caller's image into the session tempdir **before** stamping (F7) — `disk_boot`'s
  real-image boots must remain non-destructive.
- Every x86_32 integration session now boots through the chain to the disk shell; stage
  banners share `-debugcon stdio` with kernel output — verify the output matchers are
  prefix-safe (they are by construction, but this is where reality gets to disagree).
- Gate: `ROSX_ARCH=x86_32 cargo xtask test`, then full `cargo xtask verify`.

### M4 — GRUB software retirement
- `cargo xtask image x86_32` → the unified disk (C7). Delete `create_grub_image`, `GrubTooling`,
  `BootMedium::CdImage`, and their unit tests; drop xorriso/grub-mkrescue from every tool
  requirement outside `require_era_tools` (F12). Era media and scripts untouched.
- Gate: `cargo xtask check`, `cargo xtask verify`.

### M5 — Close-out
- `plans/bootloader/00-overview.md` §9: mark the x86_32 halves of *Real-kernel integration* and
  *Carrier swap* closed here (x86_64 halves stay). `docs/todo/multiboot-retirement.md`: the
  C2 sweep. `docs/todo/` pointer from §9's rom.img-as-initrd stub unchanged.
- `docs/architecture/boot-media-and-harness.md`: the x86_32 story rewritten (bootable unified
  disk, chain banners on the debug stream, boot-artifacts staging for the by-hand flow); the
  secondary-bus line stays x86_64-scoped. AGENTS.md invariant line follows.
  `arch/x86-bootloader/README.md` **Positioning** paragraph: the production-stack-untouched
  sentence becomes x86_64-only. Retro per the retro skill, reported with the final commit.

---

## 4. Risks & mitigations

| Risk | Exposure | Mitigation |
|------|----------|------------|
| Real kernel ELF rejected by the 4 KiB header window or segment overlap rules (F3) | M1 spike fails | Measured before any kernel change; a bigger `HEADER_WINDOW` is one constant if rust-lld places phdrs past 4 KiB. |
| Kernel load via the real-mode INT 13h thunk under `-cpu 486` is slow (F10) | every session | Measured at M1/M2; `PLACEMENT_WINDOW` and cluster size are the levers; `verify` runtime is the acceptance budget. |
| Stage banners pollute integration output matching | M3 | Banners are `[STAGE*]`/`[ELF]`-prefixed; matchers key on kernel lines. Proven session-by-session by the full M3 suite. |
| Slice move 63 → 2048 breaks images built by older trees | dev machines | Boot disks are build products (regenerated by `cargo xtask build`); the plan records no data-preservation law for them. Harness masters are per-session tempdirs. |
| SeaBIOS boots a carved signature-carrying index-1 disk instead of the boot disk | M3 | Explicit `bootindex=1` on the boot disk (C6). |
| Nested cargo deadlock if the builder ever shells to cargo (F9) | every session | Structurally excluded: builder consumes pre-staged files only and errors by name (C4). |
| stage2 heap: path depth, not volume size, costs (F15) | any stage2 lookup change | Kernel stays at the volume root (C5 revised); boot64 peak measured at 55.5/64 KiB — `docs/todo/stage2-bump-heap-never-frees.md` tracks the reclaim design before stage2 grows any new lookup. |

## 5. Verification law

- Every milestone: `cargo xtask check` green on its head.
- M1/M2: `cargo xtask boot-test` (24 existing checkpoints + `real-kernel-handoff32` when
  gated on); M1's manual QEMU boot recorded with measured load times.
- M3 onward: `ROSX_ARCH=x86_32` full integration suite through the chain.
- Final head (before merge): `cargo xtask verify` — gate + both-arch build + both-arch
  integration + boot-test.
- Deliberate-failure proof at least once: scribble the `ROS2` magic on a unified disk and
  watch it hang at stage1 (timeout verdict) — the boot disk is now load-bearing.

## 6. Not verified here

- Real 486 silicon: era-media behavior of the custom chain (86Box `b32` bring-up, Award INT 13h
  thunk geometry defect) stays open per AGENTS.md hardware-evidence rules; this plan's
  software path works under SeaBIOS and says nothing about period BIOSes.
- The kernel phdr position and INT-13 throughput are marked unverified above and become
  measured facts at M1.

## 7. Out of scope / follow-ups

- **rom.img as initrd (D5)** — its own successor plan; carrier shape unchanged by this series.
- **Multiboot deletion sweep** — `docs/todo/multiboot-retirement.md` (header,
  `0x2BADB002` branch, `x86_generic::multiboot`).
- **x86_64 real-kernel integration + carrier swap** — `plans/bootloader/00-overview.md` §9
  stub (higher-half link, `bootloader_api` retirement, framebuffer story).
- **Era-hardware verification** of the custom chain on 86Box/real drives, including the
  recorded thunk geometry defect.

## Friction log

    M1 | /BOOT/KERNEL.ELF (C5) OOMed 5 boot-test rows with [ERR] heap: stage2's bump heap never frees and path depth costs ~10 KiB; F10 claimed volume size was the only heap-relevant axis | MISSING-FACT | ~1h (3 boot-test runs, instrumented peaks, C5 revised)
    M1 | read boot64.console mid-suite and mistook the lm-guard-486 row's expected [ERR] CPU overwrite for a new stage2-64 defect; checkpoint rows share one console file per image | ENV-TRAP | ~15min
    M1 | committed the stage2-heap todo without running the gate; check-style rejected priority "medium" afterwards (pre-commit hook not installed in this worktree) | ENV-TRAP | 1 round
