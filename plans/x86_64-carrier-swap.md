# x86_64 Carrier Swap — `bootloader` crate out, Custom Chain In on the Unified Boot Disk

> **Status:** Complete. Written 2026-11-22 on branch `x86_64-custom-bootloader` at
> `origin/main` = `c1a1d8a` (#192, the x86_32 era verification and GRUB Legacy retirement).
> M1–M4 and V executed 2026-11-22 (execution records below; `cargo xtask verify` exit 0,
> boot-test 26/26, deliberate-failure proof done).
> **Goal:** retire the `bootloader` crate as the x86_64 boot path. The x86_64 kernel boots
> through the custom chain of `arch/x86-bootloader/` (stage1 MBR → stage2-64 → ELF64 load →
> long-mode `BootInfo` handoff) from the **unified 64 MiB FAT boot disk** — the same image the
> boot plan already mounts as `/` — mirroring `plans/x86_32-carrier-swap.md` milestone for
> milestone. This closes the two `plans/bootloader/00-overview.md` §9 stubs that kept the
> x86_64 half open: *Real-kernel integration* and *Carrier swap*.
> **Method:** every code fact below is cited to a path (and lines) re-verified against this
> tree. Facts the plan cannot prove at authoring time are marked **unverified** and routed to
> the milestone that must prove them.
> Repo law applies throughout: `system::boot_info` and the frozen handoff contract
> (`plans/bootloader/00-boot-abi.md` §3–§5) are consumed unchanged; no comments outside the
> frozen baseline (assembly excepted); commits bare imperative; every milestone lands green
> through `cargo xtask check`, and the series ends with `cargo xtask verify` on a quiet host
> (AGENTS.md contention rule) as merge evidence.

---

## 0. Goal & Non-Goals

### Goal
- `cargo xtask build x86_64` writes `target/rosx-boot-disk.img` as a **bootable unified
  disk**: MBR carrying stage1 boot code, the `ROS2`-headed **stage2-64** blob at LBA 8, one
  bootable FAT16 slice at **LBA 2048** (moved from 63) carrying every user ELF at the FAT root
  plus the chain kernel `KERNEL.ELF` at the root (x86_32 decision C5/F15 applies verbatim:
  stage2 only looks at `/KERNEL.ELF`, deeper paths OOM its bump heap) and `/boot/rom.img`.
- The x86_64 kernel grows a **chain entry**: `code-model: kernel` target, a higher-half
  linker script with `AT()` physical base, a `boot.S` trampoline that installs its own copy of
  the frozen GDT and its own boot stack and validates `RAX == BOOT_INFO_MAGIC`, and a
  `kernel_main(loader_magic, loader_info)` that consumes `system::boot_info` through the
  existing shared `kernel::platform::bootinfo::usable_blocks(…, BootArch::X86_64, …)`.
  `bootloader_api::entry_point!`/`.bootloader-config` are deleted; the `bootloader_api` and
  `bootloader` crate dependencies leave the workspace.
- Physical memory becomes **identity-addressed**: the `physical_memory_offset` consumers
  (`apic.rs`, `main.rs::build_memory_blocks`) and the `clear_nx_bits` NX workaround retire,
  because the chain's page tables identity-map `0..4 GiB` and never set NX.
- Console: the x86_64 port gains **VGA text** (`x86_generic::vga_buffer::VgaOutput`, the
  x86_32 pattern) multiplexed with the 0xE9 debug console. `framebuffer.rs`/`terminal_fonts.rs`
  are **retained in the tree but detached** (decision D1): compiled, unreferenced,
  `#[expect(dead_code)]` with a reason, revival routed to a successor plan.
- `arch/x86-runner` keeps **one boot medium**: `run_medium` hands back the unified disk
  `PathBuf` (the `BootMedium` enum died with its second variant). `BootMedium::DiskImage`
  (the carrier on `ide.1`), `create_bios_disk_image*`, `prepare_run_medium_in` and
  `attach_fat_boot_disk` are deleted; the secondary-IDE-bus invariant of
  `plans/x86_64-boot-from-disk.md` retires; both arches attach the unified disk at
  `bus=ide.0,unit=0,bootindex=1`.
- The integration harness collapses to **one session path for both arches**: copy the master
  into the session tempdir, stamp it with the arch's chain bins and `KERNEL.ELF`, disk-boot.
  Chain bins ride the pre-staged dirs `target/boot32/` and `target/boot64/` (the F9
  no-cargo-inside-cargo law).
- `cargo xtask boot-test` gains **`real-kernel-boot64`**: the real release kernel chain-boots
  off the built `target/rosx-boot-disk.img` (SKIP when the disk is absent or un-stamped, so
  the kernel-free CI `bootloader` job stays green, same law as `real-kernel-boot32`).
- `cargo xtask image x86_64` becomes the unified disk (x86_32 decision C7 analog), and CI
  publishes `target/rosx-boot-disk.img` where it publishes `rosx-x86_64.img` today.
- Docs follow each milestone; `plans/bootloader/00-overview.md` §9 *Real-kernel integration*
  and *Carrier swap* close on the x86_64 half here.

### Non-Goals
- **No rom.img-as-initrd** (decision D5 stays open, exactly as on the x86_32 swap): no
  `INITRD.BIN` on the carrier, no `include_bytes!` coupling removal, no `build.rs` assert
  deletion. Successor plan.
- **No VESA/VBE framebuffer.** The chain hands over `DisplayKind::VgaText`; growing stage2
  into a VBE mode-set is explicitly rejected for this series (16-bit-era chain surface, new
  ABI, era risk). The kept framebuffer code is the seed of that successor plan, not of this
  swap.
- **No era-hardware verification.** Nothing goes green on hardware without the AGENTS.md
  hardware-evidence rules; the §9 *Era hardware* stub stays open.
- **No stage1/stage2 code changes** — the chain is already proven in this exact role
  (`handoff64`, `handoff64-512m`, `lm-smoke`, `lm-guard-*`, `fat-boot64-media`). The single
  authorised contingency is the bump-heap fix of §4 R1, and only if it is observed failing.
- **No change to `system::boot_info`**, the handoff register/GDT contract, or any wire format.
- **No `PlatformRunner` trait.** `plans/platform-runner-abstraction.md` stays proposed; this
  swap only refreshes the facts it pins (§3.11 `rosx-x86_64.img` — deleted here) so its
  author can rebase on reality.

---

## 1. Verified baseline facts

All re-verified against `origin/main` `c1a1d8a`.

- **The 64-bit chain exists and is green today.** `arch/x86-bootloader/stage2/src/longmode.rs`
  loads `/KERNEL.ELF` (`KERNEL_PATH`, line 26), plans it with `elf64::plan_kernel_image`,
  builds the dual map with `longmode::build_page_tables` and enters via
  `enter_long_mode` (227–280): `RAX = 0x524F5358`, `RDI = &BootInfo` higher half,
  `RSP = 0xFFFF80000007F000`, CS `0x08`, DS/ES/SS `0x10`, `jmp` entry. Checkpoints
  `handoff64`, `handoff64-512m`, `lm-smoke`, `lm-guard-486/nolm` prove it in QEMU
  (`xtask/src/boot_test.rs`).
- **ELF64 load contract** (`arch/x86-bootloader/common/src/elf64.rs`): `entry >> 48` must be
  `0xFFFF` (58); PT_LOAD segments in a page-ascending virtual span with the entry inside
  (108–115); `phys_start`/`phys_end` derive from `p_paddr`. `fake-kernel-64/linker.ld` is the
  proven shape: `. = 0xFFFF800000100000;` with `AT(. - 0xFFFF800000000000)` per section,
  built with `code-model: kernel` (`arch/x86-bootloader/rosx-boot-x86_64.json`, which is
  byte-identical to `arch/x86_64/rosx.json` except that one field) and
  `-z max-page-size=0x100000` (`fake-kernel-64/build.rs`).
- **The kernel inherits its page tables and its GDT.** Nothing in `kernel/`, `arch/x86_64/`
  or `arch/x86_32/` writes `cr3` (grep: the only `mov cr3` in the tree is
  `longmode.rs:243`). The x86_64 kernel never loads a GDT; `cpu.rs::setup` (20–54) writes
  `STAR` with selectors 0x08 kcode / 0x10 kdata / 0x18 udata / 0x20 ucode — exactly the chain
  GDT (`common/src/gdt.rs`, five entries NULL/code64/data32/user_data/user_code64) and exactly
  what `plans/bootloader/00-boot-abi.md` §4 froze for this swap.
- **The chain's tables never set NX.** `common/src/pagetable.rs::fill_identity_map`: entries
  are `PRESENT | WRITABLE [| HUGE_PAGE_2MIB]`, identity `0..4 GiB` (2 MiB pages), PML4[0] and
  PML4[256] share one PDPT, so the higher-half window aliases the same physical memory.
  `cpu.rs` therefore retires both `clear_nx_bits(phys_offset)` + `walk_page_table` (145–165)
  and the `EferFlags::NO_EXECUTE_ENABLE` removal (27–33); `SCE`/`LStar`/`Star`/`SFMask` stay.
- **Physical offsets die.** `apic.rs` reaches MMIO through `phys_offset() + base` (221–241,
  static set by `set_phys_offset`, 50–54); with the identity map live the offset is the
  address. `main.rs::build_memory_blocks` (104–120) adds `phys_offset` to usable regions —
  its x86_32 counterpart (`arch/x86_32/src/main.rs:boot_info_memory` →
  `kernel::platform::bootinfo::usable_blocks`) is the replacement, and `BootArch::X86_64`
  already exists in the frozen treaty (`system/src/boot_info.rs:8–10`); stage2-64 already
  stamps it in the `BootInfo` it writes.
- **The stage2 image region is `BootloaderReclaimable` (0x7E00–0x2FFFF), the handoff stack
  and BootInfo window `Reserved`** (`common/src/memnorm.rs:34–46`, `00-boot-abi.md` §5). The
  chain GDT lives in the reclaimable region, and `00-boot-abi.md` §4 says the kernel "may
  install its own immediately" — hence decision D2.
- **x86_64 today:** `arch/x86_64/src/main.rs` is `entry_point!`-driven on `bootloader_api`
  0.11 with `Mapping::Dynamic` physical memory (51–71), a linear-framebuffer console
  (`framebuffer.rs`, fed by `boot_info.framebuffer`) multiplexed with 0xE9, and `include_bytes!`
  ROM/init (76–79). The carrier is `bootloader::BiosBoot::new(kernel_elf)`
  (`arch/x86-runner/src/lib.rs:116–120`) attached at `bus=ide.1,unit=0,bootindex=1`; the data
  FAT disk at `if=ide,index=0` is the *forward-looking* image (`xtask/src/boot_disk.rs:67–72`,
  data-only MBR, slice at LBA 63, `/boot/kernel.elf` never read).
- **x86_32 is the template:** `xtask/src/boot_disk.rs::create_bootable_disk` (74–100) —
  `stage_boot32_artifacts`, sparse 64 MiB, `chain::boot_mbr` (stage1 spliced into the rxpart
  MBR), `ROS2` blob at LBA 8, `mformat` slice at 2048, `fill_volume`, `mdir` assert; the
  harness mirrors it (`tests-integration/src/fat_disk.rs:141–235`, reading pre-staged
  `target/boot32/{stage1.bin,stage2-32.bin}`); `BootMedium::BootDisk` attaches it at
  `bus=ide.0,unit=0,bootindex=1`.
- **QEMU wiring law** (`plans/x86_64-boot-from-disk.md` O1–O3, re-proven by x86_32): bootindex
  rides the `-device`; SeaBIOS hangs on a *data-only* MBR at index 0 — after this swap the
  index-0 disk is a valid bootable MBR on both arches, so O1 no longer binds x86_64. During
  M1 the carrier keeps `bootindex=1` and still wins the boot order over the newly stamped
  index-0 disk (indexed devices boot before un-indexed ones; x86_32 proves the inverse
  topology today).
- **Harness today** (`tests-integration/src/qemu.rs`): `session_command` (361–384) and
  `spawn_with_master_disk` (63–100) branch on `arch.boot()` — x86_64: per-session carrier via
  `prepare_run_medium_in` + a separate `attach_fat_master_disk`; x86_32: stamp the master and
  boot it. No `-m` is passed anywhere (QEMU default 128 MiB), which the 64-bit kernel already
  lives with on the carrier.
- **boot-test real-kernel precedent** (`xtask/src/boot_test.rs:419–435` + `built_boot_disk`
  526–537 + `carries_boot_chain` 539–552): `BuiltBootDisk` source, `ExitVerdict::TimedOut`,
  banner requires/forbids, and the loud SKIP when the disk is absent or does not prove itself
  a chain disk. The verdict law (33/3/timeout) and the negative guards are unchanged.
- **CI** (`.github/workflows/ci.yml`, `build-artifacts.yml`): the `bootloader` job runs
  `cargo xtask boot-test` + `cargo xtask check-isa bootloader` kernel-free; uploads and the
  release asset reference `target/rosx/release/rosx-x86_64.img` (ci.yml 65, 203–210;
  build-artifacts.yml 38) — the carrier file this swap deletes — alongside
  `target/rosx-boot-disk.img` (already uploaded).
- **stage2-64 heap margin** (measured, `plans/x86_32-carrier-swap.md` F15 +
  `docs/todo/stage2-bump-heap-never-frees.md`): peak 55,552/65,536 bytes on the 16 MiB test
  media; the no-op `dealloc` bump heap ends exactly at the `0x30000` linker ASSERT. That
  todo's trigger is literally "x86_64-flavor work". See §4 R1.

---

## 2. Decisions

- **D1 — Console: VGA text now, framebuffer kept but detached** (user decision).
  `OUTPUTS` becomes `[VgaOutput, QemuDebugConsole]` like x86_32. To survive without the
  `bootloader_api` type it imports today (`framebuffer.rs:4`), `framebuffer::init` is
  re-signatured to plain dimensions + a local pixel-format enum (mechanical, same file).
  The module then stays compiled with `#[expect(dead_code, reason = "kept for the VESA
  framebuffer successor plan")]` on `mod framebuffer` — maintainer approval is this decision.
  A decision record and a `docs/todo/` entry (trigger: stage2 grows a VBE mode-set filling
  `DisplayKind::LinearFramebuffer`) accompany it. `terminal_fonts.rs` stays referenced by
  `framebuffer.rs`, so it needs no attribute.
- **D2 — `boot.S` installs the kernel's own GDT** (a `.bss` copy of the frozen five-entry
  layout) and its own 64 KiB boot stack, mirroring `arch/x86_32/src/boot.S` ("stage2's GDT is
  not part of the contract"). This decouples GDTR from the `BootloaderReclaimable` stage2
  region from the first instruction; selector values are unchanged so `STAR` and
  `process_initialization.S` need no edit. (Inheriting stage2's GDT works only as long as
  nobody ever reclaims `0x7E00..0x2FFFF` — not a law we want to depend on forever.)
- **D3 — Identity addressing.** Memory blocks, APIC MMIO and the VGA console all use identity
  addresses; the higher-half window is used only by kernel text/data (`linker.ld` VMA). No
  page-table construction, no CR3 work: the kernel keeps inheriting the chain set, as it does
  with the `bootloader` crate today.
- **D4 — Link recipe = the `fake-kernel-64` recipe, with one corrected difference (M2).**
  `arch/x86_64/rosx.json` gains `"code-model": "large"`, not `kernel`: the `kernel` and
  `medium` models address data through the 32-bit sign-extended range, and the window base
  `0xFFFF800000000000` has low-32 `0x00100000` — not sign-extending — so their 32-bit data
  relocations cannot hold a window address (`fake-kernel-64` escaped this only because it
  stores no static pointers). `large` reaches every address via `movabs`; the cost is wider
  encodings, acceptable here. `arch/x86_64/linker.ld` maps `.text/.rodata/.data/.bss` at
  `0xFFFF800000100000 AT(… - 0xFFFF800000000000)` with `kernel_end` (virtual) and
  `kernel_phys_end` (for the `usable_blocks` clip, which compares against *physical* map
  regions); `build.rs` adds `cargo::rustc-link-arg=-T…/linker.ld` and
  `-z max-page-size=0x100000`. The §9 "spike first" is satisfied by `fake-kernel-64`'s proven
  recipe plus the first real-kernel link/boot in M2, before anything else moves.
- **D5 — `rosx-boot-x86_64.json` merges into `arch/x86_64/rosx.json`** (M3). After the swap
  the two specs must never differ (the fake kernel exists to prove the real kernel's link
  shape); one file, one truth — the merged spec carries `large` (D4's correction), which the
  fake kernel also links under. `boot_image::build_fake_kernel64` and the README's layout
  table follow; `target/rosx/release/fake-kernel-64` becomes its output path.
- **D6 — `cargo xtask image x86_64` = the unified disk** (C7 analog) and CI publishes
  `target/rosx-boot-disk.img` in place of `rosx-x86_64.img` (user decision).
- **D7 — rom-as-initrd, era hardware, VESA stay out** (user decision; §0 Non-Goals).

---

## 3. Milestones

Each milestone is green through `cargo xtask check` (docs also through the eyeball gate),
committed bare-imperative, and ends with the retro skill's step-end mode.

### M1 — Media unification (data plane; x86_64 still boots via the carrier)
- `xtask/src/boot_disk.rs`: one recipe for both arches — `create_boot_disk` takes the chain
  flavor from `spec.boot` (Flat32 → `stage2-32.bin`, Long64 → `stage2-64.bin`), stamps MBR +
  `ROS2` blob + `mformat` at LBA 2048 on **both** arches, kernel at `::/KERNEL.ELF` on both
  (`fill_volume` loses its `::/boot/kernel.elf` branch); delete `create_forward_looking_disk`,
  `stamp_forward_looking_mbr`, `FORWARD_LOOKING_PARTITION_START_LBA`.
- `xtask/src/boot_image.rs`: `stage_boot32_artifacts` → `stage_chain_artifacts(flavor)`
  writing `target/boot32/` or `target/boot64/` (`target/boot64.img` is a file; no collision);
  `boot-artifacts` (`main.rs:469–480`) accepts both arches, rejecting by `spec.boot` facts
  rather than a hard-coded X86_32 check.
- `main.rs::create_flashable_image`: the x86_32 delegation becomes the only path; the
  `create_bios_disk_image` call for `cargo xtask image x86_64` is gone.
- `tests-integration/src/fat_disk.rs`: `stage_boot32_bins` → per-arch bins from the staging
  dir of the current arch; `create_bootable`/`stamp_bootable`/`create_boot_master` otherwise
  unchanged (inert until M2 — the x86_64 harness still carrier-boots and ignores them).
- CI upload lines: drop `rosx-x86_64.img`, publish `target/rosx-boot-disk.img` (x86_64 job).
- Proof: `cargo xtask check`; `cargo xtask build x86_64` produces a chain-stamped
  `rosx-boot-disk.img` (an x86_32-style `mdir`/hexselfcheck via `carries_boot_chain` logic);
  `cargo xtask run x86_64` still boots the carrier (bootindex law above); full integration
  x86_64 + x86_32 unchanged; `cargo xtask boot-test` unchanged (32-bit rows only).

### M2 — The flip: kernel on the chain, runner and harness follow (one commit)
The kernel entry, the runner medium, the harness session path and the `xtask run`/`launch`
branch move **together** — between them the tree does not boot.
- **Link spike first** (D4, unverified → proven here): add the code model + `linker.ld` +
  `build.rs` link-args; `cargo xtask build x86_64`; `readelf -lW` must show the
  `0xFFFF800000…` AT `0x001…` span pattern with entry in `.text`; keep orphan sections
  explicit (discard `.eh_frame/.comment/.note/.debug` as `fake-kernel-64/linker.ld` does).
- `arch/x86_64/src/boot.S` (new, mandatory register comments per style law): `_start` —
  cli, build the 5-entry GDT pointer to a `.bss` table (frozen layout), `lgdt`, far-reload
  CS, load DS/ES/SS/FS/GS with `0x10`, load a 16-aligned `.bss` boot stack, park `RAX`/`RDI`
  as `kernel_main(magic, info)` arguments (`call 2f` / `pop` for the runtime-RIP proof is not
  needed here; `fake-kernel-64` keeps it for its contract, the real kernel does not), call
  `kernel_main`, then a `kernel_returned` halt trap.
- `arch/x86_64/src/main.rs`: delete `bootloader_api` imports, `BootloaderConfig`,
  `entry_point!`, `build_memory_blocks`; `kernel_main(loader_magic: u64, loader_info: u64)`
  mirrors x86_32: `vga_buffer::init(0)` + `setup_default_output` first, magic check with the
  same "unknown boot magic" wording, `boot_info_memory` via
  `usable_blocks(addr, BootArch::X86_64, kernel_phys_end, …)` printing the same
  `[BOOTINFO] regions=…` line, then the unchanged bootstrap/setup/ROM/init/boot-plan body.
- `cpu.rs`: delete `clear_nx_bits`/`walk_page_table` and the NXE-removal comment+call;
  `apic.rs`: delete `PHYS_OFFSET` plumbing (direct identity MMIO); `framebuffer.rs`:
  re-signatured `init` + `#[expect(dead_code)]` per D1; `mod framebuffer;` no longer in
  `OUTPUTS`.
- `arch/x86_64/Cargo.toml`: delete `bootloader_api`; `arch/x86-runner/Cargo.toml`: delete
  `bootloader`.
- `arch/x86-runner/src/lib.rs`: `BootMedium` is deleted; `run_medium`
  derives `target/<kernel_dir_name>-boot-disk.img` for both arches and refuses by name when
  absent; delete `prepare_run_medium`, `prepare_run_medium_in`, `create_bios_disk_image*`,
  `bios_disk_image_name/path`, `attach_fat_boot_disk`; `qemu_command` single medium branch
  (`bus=ide.0,unit=0,bootindex=1`); unit tests rewritten to the two-arch single shape.
- `tests-integration/src/qemu.rs`: `session_command` and `spawn_with_master_disk` lose their
  x86_64 branches (copy-then-stamp before stamping law intact); delete
  `attach_fat_master_disk`.
- `xtask`: `launch`/`run` lose the carrier + FAT double-attach branch; `run_tests` stages the
  per-arch flavor; `xtask/src/boot_test.rs` — generalize `built_boot_disk`/`carries_boot_chain`
  over the arch's disk path and add **`real-kernel-boot64`**: machine `BIOS_64` (qemu64,
  128M — the boot-test default the fake kernel already boots; integration's default-128M
  precedent), `BuiltBootDisk` (64-bit), `TimedOut` verdict, requires `[STAGE1] OK`,
  `[STAGE2_64] A20=OK THUNK=OK`, `[ELF] arch=X86_64`, `[BOOTINFO] regions=`,
  `[KERNEL] Bootstrapped`, `[KERNEL] Initializing`, `[KERNEL] Starting`; forbids
  `!!! PANIC !!!` and `unknown boot magic`. **Unverified** until run: the 128 MiB budget for
  the full 64-bit boot and the stage-64 heap peak on production media (§4 R1).
- Proof: `cargo xtask check`; `cargo xtask boot-test` — both real-kernel rows PASS;
  **full integration on x86_64** (now chain-booting sessions) and x86_32;
  `cargo xtask run x86_64` reaches the disk shell; `cargo xtask verify` as the milestone's
  merge-evidence tail.

### M3 — Simplification sweep
- Merge `arch/x86-bootloader/rosx-boot-x86_64.json` into `arch/x86_64/rosx.json` (D5);
  repoint `boot_image.rs:35` `FAKE_KERNEL_64_TARGET_SPEC` and the README layout line.
- Retire `ArchSpec::fat_boot_disk` if no consumer survives; sweep `x86-runner` for exports
  without a production caller (AGENTS.md no-dead-surface law).
- Debug tooling: `arch/x86_64/debug-qemu.sh`, `start-qemu-background.sh`,
  `ensure-qemu-running.sh`, `DEBUG_SETUP.md`, `gdb-init.gdb` → unified disk + higher-half
  symbols.

### M4 — Docs and records
- `AGENTS.md`: replace the "No bootimage: the x86_64 disk image comes from `arch/x86-runner`
  (the `bootloader` crate)" bullet — both arches chain-boot the unified disk that
  `arch/x86-runner` packs from `arch/x86-bootloader`'s stage1/stage2.
- `docs/architecture/boot-media-and-harness.md`: rewrite the x86_64 bullets to the unified
  disk; **retire the secondary-IDE-bus invariant**; update the integration-harness bullet
  (one session path); keep the O1-derived facts that still bind (bootindex on the device).
- `arch/x86-bootloader/README.md`: positioning paragraph (x86_64 production boots through the
  chain), checkpoint count/table gains `real-kernel-boot64`, layout note for the merged
  target spec.
- `plans/bootloader/00-overview.md` §9: mark the x86_64 halves of *Real-kernel integration*
  and *Carrier swap* closed (the framebuffer sentence: decided VGA text, VESA successor
  recorded); *rom.img as initrd* and *Era hardware* stay open.
- `plans/platform-runner-abstraction.md`: fact-refresh note (artifact table,
  `prepare_run_medium_in`/`DiskImage` are gone).
- `docs/decisions/`: retirement of the `bootloader`/`bootloader_api` dependencies; the
  VGA-text-with-kept-framebuffer decision (D1). `docs/todo/`: VESA-framebuffer revival
  successor (with the rom-as-initrd D5 stub cross-referenced, untouched).
- Ratchet: unsafe counts fall with `clear_nx_bits`/`walk_page_table`; refresh
  `xtask/style-baseline.txt` down via the documented `--update-baseline` path (a decrease,
  no decision record needed).

### V — Merge evidence
- Quiet-host check first (`pgrep -f 'cargo test|xtask test|qemu-system'`, load vs `nproc`).
- `cargo xtask verify` (gate + both-arch integration + boot-test).
- Deliberate-failure proof (the carrier-swap law): scribble the `ROS2` magic on
  `target/rosx-boot-disk.img`, boot, expect stage1 `[ERR] disk` and a timed-out verdict;
  restore.
- Retro (full-workflow mode) with the final report.

---

## 4. Contingencies and risks

- **R1 — stage-64 bump heap on production media** (the one authorised stage2 change).
  55,552/65,536 measured on sparse test media; the production root adds ~45 8.3-name entries
  plus `/boot` (directory-read `Vec`s never free — `docs/todo/stage2-bump-heap-never-frees.md`,
  whose trigger is exactly this work). If `real-kernel-boot64` shows `[ERR] heap`, land the
  todo's cure (LIFO tail-free bump or a reusable directory buffer) as its own commit inside
  M2 before the flip can go green; it needs no ABI change.
- **R2 — rust-lld on the full kernel.** `fake-kernel-64` proves the recipe small; the real
  kernel's section mix is bigger. Mitigated by the M2 spike-first rule: link + `readelf`
  audit before the flip is wired, so a linker-script surprise costs nothing.
- **R3 — 128 MiB boot.** The 64-bit kernel never ran under the chain at this size; the
  `handoff64` family runs at 128 MiB and integration defaults to 128 MiB, but the full boot
  to shell is new. If placement refuses (`[ERR] mem`), the honest fix is the checkpoint's
  machine RAM (`BIOS_64_512`), not a media change — recorded either way in the M2 execution
  note.
- **R4 — era silicon.** VGA (`0xB8000`) and APIC MMIO through 2 MiB identity pages are
  WB-mapped (PAT/default) — fine in QEMU, suspect on silicon; the Award-BIOS INT 13h thunk
  defect from the b32 era run still applies to the 64-bit chain too. Both stay in the §9
  *Era hardware* open stub; nothing here claims hardware.
- **R5 — flakiness.** M2's integration evidence obeys the AGENTS.md quiet-host rule; a
  contention failure is not a verdict.

## 5. What closes / what stays open

Closes: §9 *Real-kernel integration* (x86_64 half) and *Carrier swap* (x86_64 half) of
`plans/bootloader/00-overview.md`; the forward-looking-artifact sentence of
`plans/x86_64-boot-from-disk.md`; the `bootloader`-crate dependency.
Stays open: D5 rom.img-as-initrd; era-hardware verification (both arches); VESA framebuffer
successor; `plans/platform-runner-abstraction.md` (trait plan, now on a simpler base).

---

## Friction log

    M1 | x86_64 integration run piped to tail -20, cargo's exit code masked; full suite rerun (~30 min) to capture exit-code evidence | ENV-TRAP | 1 round (~30 min)
    M1 | `cargo xtask check` output piped through grep|head -30 cut the verdict line; rerun needed to confirm all-green | ENV-TRAP | 1 round (~4 min cached)
    M1 | ad-hoc python MBR stamp check raised struct.unpack ValueError (wrong entry format); fixed second run | TASK | 0
    M2 | boot.S absolute data references emitted R_X86_64_32 relocations; the `kernel`-model link failure was misread as Rust codegen before the asm was blamed (fake-kernel-64 hid both defects) | TASK | 2 link rounds (~5 min)
    M2 | first chain boot reached kernel_main with magic 0x10: boot.S reused rax — the magic carrier — for the far-reload target and the selector load, ignoring the 32-bit template's register discipline | TASK | 1 boot round
    M3 | the D5 merge linked fake-kernel-64 under `large` with `p_paddr == p_vaddr` segments: neither linker.ld collected the `.ltext/.lrodata/.ldata/.lbss` orphan families the large model emits (the real kernel was accidentally rescued by boot.S's non-empty `.text` inheriting the AT region); caught by the ELF audit before any boot | MISSING-FACT | 1 rebuild round; both scripts collect them now, fact lands in the code-model decision record (M4)
    M4 | decision-record filenames `x86_64-*.md` rejected by the records validator (kebab-case law already in AGENTS.md); renamed to `x86-64-*` next round | MISSING-FACT | 1 gate round

Lessons:

- **[M1] Process (no gate or doc covers it):** a suite's output that was truncated is a
  suite that never ran — derive verdicts only from captured files and exit codes. A 40-minute
  `cargo xtask verify` that died mid-run hid behind a tail-piped output and made `rosx-x86_32`
  look like it passed; the same trap fired on `cargo xtask test`. Approved lesson (2026-11-22,
  landed with its ledger row): **ALWAYS capture long-running suite output to a file with its
  exit code; NEVER derive a verdict from piped or truncated output.**
- **[M2+M3] Covered by existing docs and gates, with a fact gained:** the large code model
  emits `.ltext/.lrodata/.ldata/.lbss` orphan families and both linker scripts collect them
  (fact landed in `docs/decisions/x86-64-link-code-model-large.md` and the link recipe in
  `docs/architecture/boot-media-and-harness.md`); the handoff failure classes this plan hit —
  a selector or far-target landing in the magic-carrying register (`rax`), a 32-bit data
  relocation under a non-sign-extended window base — stay documented in
  `docs/architecture/boot-media-and-harness.md`.

M1 execution record (2026-11-22, commit `3cbfb8b`): the re-stamped index-0 disk stayed out of
the boot order — the bootindex=1 carrier won, SeaBIOS never touched the new bootable MBR — and
the kernel mounted `ata0d0p0` from the LBA-2048 slice to the disk shell (boot-plan green,
`rose>` reached, zero PANIC on a headless reproduction of the `cargo xtask run x86_64`
topology). Gate, both integration suites (40/40 each) and all 25 boot-test rows green.

M2 execution record (2026-11-22, commit `9052850`): the real kernel now boots from the unified
disk through stage1→stage2-64 (standalone: `[BOOTINFO] regions=11`, mount `ata0d0p0`, `rose>`,
zero PANIC; no `[ERR] heap` — R1 did not fire). ELF audit: entry `0xffff800000100000`, every
PT_LOAD at `p_paddr = p_vaddr − 0xFFFF800000000000`, `kernel_phys_end` correct.
x86_64 never needed a TSS — `syscall.S` returns to ring 0 ("we don't support Ring 3 yet") and
the carrier never loaded one either, so the chain inherits an already-TSS-free kernel.
`real-kernel-boot64` added as boot-test row 26; gate green with the unsafe/comments ratchet
down; both integration suites 40/40 (all x86_64 sessions now boot chain-stamped masters).

M3 execution record (2026-11-22, commit `1afa661`): one 64-bit target spec —
`arch/x86_64/rosx.json` — now links kernel and fake-kernel-64 alike. The merge turned the
real kernel's accidental orphan survival (rust-lld placing `.ltext*` next to a non-empty
AT()'d `.text` region) into a deterministic recipe: both `linker.ld`s collect
`.ltext*/.lrodata*/.ldata*/.lbss*` explicitly, and the fake kernel's `p_paddr` audit is green
again under `large`. `fat_boot_disk` died with its last consumer; runner exports narrowed to
what callers use; the debug tooling boots the unified disk from reset with higher-half
symbols. Gate green, boot-test 26/26.

M4 execution record (2026-11-22, commit `0074b45`): the docs now match the machine — AGENTS.md,
`docs/architecture/boot-media-and-harness.md` (unified-disk story for both ports,
secondary-bus invariant retired, link recipe spelled out), the release-boot recipes in the
root README, the boot-reader README positioning with a 26-row checkpoint table (the
`real-kernel-boot32` row had never been listed), §9 x86_64 halves closed, the runner-abstraction
plan refreshed. Decision records `bootloader-crate-retirement`, `x86-64-console-vga-text`,
`x86-64-link-code-model-large` and todo `vesa-framebuffer-successor.md` filed; the records
validator keeps filenames kebab.

V execution record (2026-11-22): quiet-host verified (load 0.14, no live suites) before
`cargo xtask verify` — exit 0: gate green, x86_64 build + integration, x86_32 build +
integration, boot-test 26/26, 208 suite-result lines zero failures. Deliberate-failure proof:
scribbling `ROS2`→`XXXX` at LBA 8 of the production `target/rosx-boot-disk.img` left stage1
printing `[ERR] disk 08` with zero `[STAGE2_64]` output and a timed-out verdict — the disk is
the only medium — and the disk restored bit-identical afterwards.
