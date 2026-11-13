# GRUB Boot for Both x86 Ports — Plug-and-Play Release Media

> **Status:** Proposed 2026-11-13 on branch `boot-grub-both-ports` (investigation only; no code changed).
> **Goal:** both x86 ports boot through GRUB — no QEMU `-kernel`, no bespoke boot stages — so the
> distributed release images boot plug-and-play on any VM (attach one file, press start) and on real hardware.
> **Method:** every code fact is cited to a path re-verified against HEAD `adbcedf`; every loader
> fact below was verified against upstream GRUB 2.14 / QEMU 10.2.1 sources and where feasible
> reproduced end-to-end on this machine with a minimal ELF64 multiboot stub.
> Repo law applies throughout: `kernel/` stays `no_std` and arch-agnostic, no comments except
> assembly, unit tests for kernel modules, no test-only production surface, bare commit subjects.

---

## 0. Verdict

**Feasible on both ports, with one asymmetry.**

- **x86_32** is already a Multiboot v1 kernel and already boots under GRUB 2 (CI ISO,
  `cargo xtask image x86_32`) and GRUB Legacy 0.97-67 (`cargo xtask era-image x86_32`, proven to
  `rose>` on emulated 486 hardware via 86Box). What is missing is only the *packaging*: the
  current artifacts boot to the ROM shell because no FAT root volume rides along. The
  plug-and-play artifact is a GRUB-installed HDD image whose single FAT slice carries the kernel
  *and every app ELF* — the era-HDD recipe and `boot_disk.rs` merged (§3).

- **x86_64** cannot be loaded by GRUB today: it speaks the `bootloader` crate's bespoke protocol
  (`entry_point!` → `BootInfo`, `arch/x86_64/src/main.rs:17-57`). The fix is a Multiboot v1
  bootstrap in the kernel (header + own 32→64-bit long-mode transition, GRUB never enters long
  mode for you) — a well-trodden, self-contained change (§2). GRUB 2 loads Multiboot ELF64
  kernels natively (verified, §1 F6); GRUB Legacy does not (F7), so the era channel stays
  x86_32-only as designed.

- One consequence to accept: **QEMU's `-kernel` cannot load a Multiboot ELF64 kernel** (verified,
  F5). Today's x86_64 dev flow does not use `-kernel` anyway (it boots the BiosBoot carrier
  disk), so nothing is lost; after the change the dev run boots the same GRUB ISO the release
  ships, which is strictly better dogfooding. x86_32 keeps `-kernel` for fast integration runs.

---

## 1. Verified facts

### Code facts (against `adbcedf`)

- **F1** x86_32 entry is already loader-neutral Multiboot v1: header + 32-bit entry + own GDT +
  `(magic, mbi_ptr)` handoff to `kernel_main` (`arch/x86_32/src/boot.S`,
  `arch/x86_32/src/main.rs:40-51`), memory map through the shared parser
  (`x86_generic::multiboot::usable_memory`, `arch/x86-generic/src/multiboot.rs:42`).
  Same parser is `usize`-generic and reusable on 64-bit unchanged.
- **F2** x86_64 depends on the `bootloader` crate at exactly three points, all in `arch/x86_64`:
  `main.rs` (`entry_point!`, `BootInfo`, memory regions, `physical_memory_offset`),
  `framebuffer.rs` (`FrameBufferInfo`), `cpu.rs::clear_nx_bits` (strips NX from the page tables
  the crate's stage built; `arch/x86_64/src/cpu.rs:145-164`). Nothing in `kernel/`, `system/`,
  `usrlib/`, or `arch/x86-generic/` references it. The x86_64 kernel has **no linker script** —
  default lld layout; the crate's stage3 loads the ELF and builds the page tables.
- **F3** Media tooling today: `x86_runner::create_bios_disk_image` (bootloader-crate `BiosBoot`)
  for x86_64 `image`+`run`; `x86_runner::create_grub_image` (grub-mkrescue ISO staging
  `/boot/kernel` + `multiboot` cfg, `arch/x86-runner/src/lib.rs:108-131`) for x86_32 `image`;
  GRUB-Legacy era scripts in `arch/x86-86box/scripts/` (docker `grub --batch` setup against a
  plain file, no loop devices). The FAT boot-disk recipe (`xtask/src/boot_disk.rs`,
  rxpart-stamped MBR + FAT16 slice at LBA 63 + all app ELFs at root) produces an MBR with **no
  boot code** — it is a data volume, not bootable media.
- **F4** Release artifacts (`build-artifacts.yml`): x86_64 uploads the BiosBoot carrier +
  separate FAT boot disk (two files the user must attach in order); x86_32 uploads kernel +
  rom + GRUB ISO. The ISOs stage only the kernel, so they boot past a failed
  `mount ata0d0p0 /` into the **ROM shell**, not the disk shell.
- **F5** The boot plan needs nothing new: `run rom:mount ata0d0p0 /` names exactly the slice a
  GRUB-installed FAT HDD image exposes through blockd curation (MBR type 0x06 → `ata0d0p0`).
  A CD-ROM can never carry the root volume: `atad` speaks ATA only, no ATAPI PACKET.

### Loader facts (upstream GRUB 2.14, QEMU 10.2.1, 2026-11-13)

- **F6** *GRUB 2 (BIOS, i386-pc) loads Multiboot v1 ELF64 kernels.* Its multiboot loader compiles
  for both ELF classes (`grub_multiboot_load_elf64`,
  `grub-core/loader/multiboot.c:226-231`, `grub-core/loader/multiboot_elfxx.c`), expects
  `e_machine == EM_X86_64` for class 64 and rejects segments crossing 4 GiB. **Reproduced:**
  a hand-built ELF64 stub (`0x1BADB002` header, `.code32` entry at 1 MiB) in a grub-mkrescue ISO
  booted under `qemu-system-x86_64` and printed on the debug console; the same stub on a
  grub-mkrescue-produced *hybrid* ISO boots whether attached as CD or written to a disk.
- **F7** *GRUB Legacy 0.97 does not support ELF64 multiboot kernels* (ELF32 only — matches
  upstream guidance, e.g. the GNU Hurd docs). Era media therefore remains an x86_32 channel.
- **F8** *QEMU's userspace `-kernel` multiboot loader rejects ELF64*: it loads with
  `elf_machine = I386_ELF_MACHINE` (`hw/i386/multiboot.c:199-201`), and the generic loader's
  machine check then fails an `EM_X86_64` image (`include/hw/loader/elf_ops.h.inc`, default
  case). **Reproduced:** `-kernel stub64.elf` → `Cannot load x86-64 image, give a 32bit one.`
- **F9** *GRUB does not switch to long mode*: a Multiboot kernel entered from GRUB (v1 or v2) is
  in 32-bit protected mode, paging off, flat code — the kernel owns the PAE/LME/PG sequence.
  This is what `arch/x86_32/src/boot.S` already handles one mode down.
- **F10** Unprivileged GRUB media building: **grub-mkrescue ISOs need nothing special** (no root,
  no loop devices; CI already installs `grub-pc-bin grub-common xorriso mtools`). A **GRUB
  hand-written HDD image (`boot.img` + dd'd `core.img`) is a trap**: reproduced the core loading
  (early config ran) while GRUB's runtime disk layer saw no disks at all — the installer
  (`grub-bios-setup`) patches identity fields beyond the raw bytes, and `grub-bios-setup` itself
  needs host-device access (fails in sandboxes; `losetup -P` + `grub-install` on a machine with
  sudo — e.g. the GitHub runners — is the standard recipe). Packaging must therefore go through
  `grub-install`/`grub-bios-setup`, never manual sector writes.
- **F11** Multiboot v1 mmap entries arrive in the packed layout the existing parser reads
  (`size@0, base@4, len@12, type@20`, stride `size+4`): that is the struct both GRUB 2
  (`include/multiboot.h:230-240`, `u64` with 32-bit alignment) and the SeaBIOS/QEMU path
  produce, and the x86_32 port is measured working through both loaders. The spec text's
  "unused@4 / base@8" variant must not be "fixed" into the parser without a loader-side
  fixture test.

---

## 2. x86_64: Multiboot bootstrap (replaces the `bootloader` crate path)

Kernel-side, all changes confined to `arch/x86_64`:

1. **`boot.S`** (new): Multiboot v1 header (magic/flags/checksum, `ARCH_x86_64` header not
   needed — `0x1BADB002` with `e_machine == EM_X86_64` is what GRUB matches on, F6/F1) + 32-bit
   `_start`: `cli`, own GDT with 64-bit code/data descriptors (the x86_32 pattern at
   `arch/x86_32/src/boot.S:9-25`, extended), build boot page tables in `.bss`, set `PAE` in CR4,
   `LME|LMA` in EFER, `PG` in CR0, far jump to 64-bit `_start_long`, set a 64 KiB boot stack,
   then call `kernel_main(magic, mbi)` with the same `(u32, u32)` signature x86_32 uses.
2. **Memory model (decision D2)** — recommended: **pure identity map with 2 MiB pages** covering
   the first N GiB (kernel linked at 1 MiB like x86_32). Consequences:
   - new `arch/x86_64/linker.ld` (≈ x86_32's, multiboot section first, `kernel_end`);
   - `physical_memory_offset` becomes 0: `apic::set_phys_offset(0)`, `build_memory_blocks`
     consumes `x86_generic::multiboot::usable_memory` output directly (F1/F11);
   - `cpu::clear_nx_bits` deleted (our tables never set NX — the function only existed because
     the crate mapped the kernel NX, `arch/x86_64/src/cpu.rs:26`);
   - the kernel keeps running on the boot page tables exactly as it runs on the crate's tables
     today (no per-task tables exist either way).
   Alternative (higher-half link + `phys_offset` mirror of the old model) works — Linux-style
   `AT()` load addresses plus a trampoline jump into the higher half — at strictly more
   complexity for zero current benefit.
3. **Console**: `x86_generic::vga_buffer` replaces `framebuffer::FramebufferOutput` (parity with
   x86_32; Multiboot v1 consoles are text — GRUB leaves VGA text mode 3 after `boot`).
   `framebuffer.rs`/`terminal_fonts.rs` retire with this step; a Multiboot2 framebuffer revival
   is milestone M5.
4. **`main.rs`**: `extern "C" kernel_main(magic, mbi)` shaped like x86_32's
   (`arch/x86_32/src/main.rs:43-77`); drop `bootloader_api` from `arch/x86_64/Cargo.toml`.
   IDT/PIC/syscall/user-task machinery is untouched.

Runner/tooling-side:

5. `x86_runner`: `run_medium(X86_64)` becomes an ISO medium built with `create_grub_image`
   (already handles the `multiboot` cfg, F3); the `bootloader` dependency leaves
   `arch/x86-runner/Cargo.toml`; `BootMedium::DiskImage` remains for the M3 HDD medium. Dev
   `cargo run` boots that ISO (≈ 1–2 s of GRUB overhead over today's carrier boot).

Tests follow the change: unit test for the linker-visible boot memory trim (x86_32's
`trim_to_safe_memory` analogue), and the existing integration suites keep passing through the
new medium with zero test-body changes (`prepare_run_medium_in` already builds media per session,
`tests-integration/src/qemu.rs:68`).

## 3. Both ports: plug-and-play release media

The single-file artifact: **a GRUB-installed HDD image** — `boot_disk.rs` layout (rxpart-shaped
MBR + one FAT16 slice) carrying `/boot/kernel`, `/boot/grub/grub.cfg`, and every `USER_APPS` ELF
at the FAT root, with GRUB boot code installed by `grub-install`. Attach one `.img` to any VM →
BIOS → GRUB → kernel → `mount ata0d0p0 /` succeeds → **disk shell**, apps and all; the same file
written to a real IDE/SATA disk or USB stick boots identically on BIOS hardware.

Build path (F10 — no hand-rolled sectors):
- CI (`build-artifacts.yml`, ubuntu runners, sudo available): `losetup -f --show -P img` →
  `grub-install --target=i386-pc --boot-directory=<mnt> /dev/loopX` → `kpartx -du`. Runs beside
  the existing mtools staging step.
- Local (`cargo xtask image`): ISO via grub-mkrescue (zero privileges, already the x86_32
  pattern); the HDD image additionally runs through a pinned container step when
  `grub-bios-setup` cannot probe host devices on the machine — the same trust pattern
  `arch/x86-86box/scripts/build-legacy-hdd.sh` already establishes for the era flow.

Release uploads become per port: `rosx-<arch>.iso` (hybrid; CD or disk) + `rosx-<arch>.hdd.img`
(the single-file plug-and-play artifact). The x86_64 BiosBoot carrier and the loose "attach two
disks in order" story retire.

## 4. Decisions for the maintainer

- **D1 — Multiboot v1 vs v2 for x86_64.** Recommendation: **v1** — reuses the proven MBI parser
  and the x86_32 handoff contract verbatim; v2 buys only the framebuffer tag (M5) and a bigger
  header/loop parser. (v1 and v2 headers can coexist later without touching the v1 path.)
- **D2 — x86_64 memory model.** Recommendation: **identity map** (§2.2).
- **D3 — Retire the `bootloader` crate entirely?** Recommendation: yes — one standard boot
  interface for both ports; keeping two boot paths on x86_64 doubles the boot-surface audit.
- **D4 — x86_64 dev/test medium.** Booting the GRUB ISO (F8 closes `-kernel` permanently for
  ELF64). If `-kernel` speed ever matters, a PVH note (`XEN_ELFNOTE_PHYS32_ENTRY`) can coexist
  with the multiboot header later; not planned here.

## 5. Milestones

- **M1 — x86_64 multiboot kernel.** `boot.S` + `linker.ld` + `main.rs` entry swap + vga text
  output + memory-map reuse; delete the `bootloader_api` dependency. Acceptance: hand-built
  `grub-mkrescue` ISO boots to `rose>` (ROM shell) in `qemu-system-x86_64`; `check-isa` N/A;
  unit tests green.
- **M2 — tooling parity.** `run`/`image` for x86_64 on the GRUB ISO; `x86-runner` sheds
  `bootloader`; integration suites boot through `prepare_run_medium_in` unchanged on both arches.
- **M3 — single-file HDD images.** Merged recipe (boot_disk layout + grub-install via CI
  losetup / local container fallback); release workflow uploads ISO + HDD for both ports.
  Acceptance: QEMU with *only* the attached `.img` reaches the **disk shell** on both arches
  (this is the "plug and play" bar), plus the x86_32 ISO/HDD keep working unchanged.
- **M4 — tests & docs.** x86_64 disk-boot integration test (mirrors the x86_32-only `disk_boot`
  suite); AGENTS.md boot section rewritten around GRUB media; era channel documented as
  x86_32-only *by loader capability* (F7), not by choice.
- **M5 — (deferred, optional)** Multiboot2 header + framebuffer tag to restore the graphical
  console on x86_64 under GRUB, VGA-text fallback preserved.

## 6. Risks / open items

- GRUB writes the MBR boot code over the rxpart stamp area (`encode_mbr`,
  `xtask/src/boot_disk.rs`): partition bytes are preserved by GRUB's stage1/installer (the era
  HDD has done exactly this and boots), but the curation test should re-run against a
  grub-installed image in M3.
- mmap entry layout (F11): the parser matches GRUB/SeaBIOS reality; pin that with a fixture test
  before reuse on 64-bit, so a future spec-lawyering "fix" cannot silently break boot.
- Memory > 4 GiB on x86_64: identity map covers a chosen GiB span with 2 MiB pages; the MBI mmap
  parser caps at 32 regions and rejects what exceeds `usize::MAX`; document the covered span in
  the trampoline (same cap posture `MAX_BOOT_MEMORY_REGIONS` already takes).
- The manual `boot.img`/`core.img` dd recipe is explicitly rejected (F10): any packaging PR that
  reintroduces hand-written sector patching must fail review on this evidence.
- `cargo xtask era-image x86_32` stays as-is; nothing in the 86Box evidence chain changes.
