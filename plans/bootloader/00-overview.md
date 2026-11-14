# Custom x86 BIOS Bootloader — Overview & Ground Rules

> **Status:** Proposed. Written 2026-11-14 on branch `custom-bios-bootloader`, HEAD `adbcedf`.
> **Goal:** a custom multi-stage x86 BIOS bootloader, built and proven entirely in QEMU under
> `arch/x86-bootloader/`, that loads an `ELF32` (`EM_386`) kernel in 32-bit protected mode
> (paging off) on i486-class CPUs and an `ELF64` (`EM_X86_64`) kernel in 64-bit long mode mapped
> in the higher half (`0xFFFF_8000_0000_0000`), handing both a `system::boot_info` `BootInfo`.
> **Method:** every fact about the current tree is cited to a path re-verified against `adbcedf`;
> every exit-code fact was measured on this machine (QEMU 10.2.1). Repo law applies throughout:
> **no comments ever** (assembly excepted), zero warnings on every build surface, no test-only
> production surface, `cargo build --workspace` stays forbidden, commit subjects are bare
> imperative (no prefix), and every phase lands as tiny approved increments.

---

## 1. What this plan is, and what it deliberately is not

Build the bootloader as a **standalone, self-contained deliverable**: new crates under
`arch/x86-bootloader/`, its own disk-image recipes, its own QEMU test harness, its own CI job.
"Complete" means an automated `cargo xtask boot-test` passes every phase checkpoint end to end,
culminating in two fake kernels that receive `BootInfo`, verify it, CRC-check an `INITRD.BIN`,
print `kernel loaded` with the matching checksum to `COM1`, and exit QEMU with the success code.

**Scope fence.** The existing boot stack is **untouched and stays green** for the whole plan:

- x86_64: `bootloader` crate 0.11 (`BiosBoot`) + `arch/x86-runner` carrier — unchanged.
- x86_32: Multiboot v1 header in `boot.S`, `-kernel` booting, GRUB ISO, GRUB Legacy era media —
  unchanged.
- `tests-integration`, boot plan, ROM embedding (`include_bytes!`) — unchanged.

None of the new crates enter any existing boot path. This plan ships a proven engine plus a frozen
ABI; wiring the engine into the real kernels is the follow-on plan (§8).

## 2. Decision record

| # | Decision | Resolution |
|---|----------|------------|
| D1 | Scope | Stop at fake kernels; real-kernel integration is a separate successor plan. |
| D2 | Boot ABI home | `system::boot_info` module — the treaty crate already owns syscall numbers, ROM layout, and boot-plan grammar; no second treaty crate (`rosx-boot-abi` is **rejected**). See `00-boot-abi.md`. |
| D3 | FAT reader | Reuse the workspace engines: `rxfat` over `fs-core::BlockDevice` implemented on the real-mode INT 13 thunk, with a tiny bump allocator. Hand-rolled FAT parsing is **rejected**; workspace crates are not the "external bootloader crates" the self-containment rule excludes. |
| D4 | ISA-gate strategy for long-mode code | **Two stage2 builds** from one crate behind a build feature: `stage2-32` audited by the existing `check-isa --stages` deny-list verbatim; `stage2-64` audited by the same deny-list plus exactly `{cpuid, rdmsr, wrmsr}`. |
| D5 | Initrd semantics | Initrd is a test payload for this plan. The approved direction afterward: the loader reads `/boot/rom.img` from disk as the real initrd and the kernel registers the ROM from it, retiring the `include_bytes!` build-order coupling. Follow-on, not here. |
| D6 | Placement | Everything lives under `arch/x86-bootloader/` (workspace crates + asm + linker scripts + target JSON), mirroring the self-contained `arch/x86-86box` precedent. |

## 3. Hard constraints

1. **i486 floor for all 32-bit artifacts.** `stage2-32`, `stage2-64`'s pre-long-mode code, and
   `fake-kernel-32` compile against a new `arch/x86-bootloader/rosx-boot-i486.json` target that
   copies the pinned feature string of `arch/x86_32/rosx-i486.json`
   (`-mmx,-sse,-sse2,-cmov,-fxsr,-cx8`). Never emit i686+ instructions in code reachable on a
   486. The guarded long-mode path (`CPUID` probe, `EFER` MSRs) lives only in `stage2-64`.
2. **No floating point on any 32-bit artifact**; atomics ≤ 32 bits; no `cpuid`-probed features on
   the 486 path (the LM probe is x86_64-only and guarded).
3. **No external crates.** Allowed dependencies: `system`, `fs-core`, `rxfat` (all workspace,
   zero third-party deps) and `core`/`alloc`.
4. **Machine guards, not hopes.** The existing `check-isa` machinery is extended to audit every
   shipped bootloader binary (§6). A regression must fail CI, never a board.
5. **The ABI is frozen at the end of this plan** (§8): the handoff registers and the x86_64 GDT
   selector layout written in `00-boot-abi.md` must survive into the integration plan unchanged.

## 4. Repository layout (deliverable)

```
arch/x86-bootloader/
├── README.md                 # runbook: build each artifact, run each test by hand
├── rosx-boot-i486.json       # target spec: copy of rosx-i486.json feature pins
├── stage1/
│   ├── boot.s                # .code16, MBR-resident, code ≤ 446 bytes + table + 0x55AA
│   └── stub2.s               # [STAGE2_16] era stub, replaced by the stage2 crate at phase 4
├── stage2/                   # crate; features stage-32 / stage-64 select the boot path
│   ├── linker.ld             # .text.entry at 0x7E00, BSS bounds exported
│   └── src/ ...
├── common/                   # rosx-boot-common: 16550 serial writer, hex formatting, CRC32
├── fake-kernel-32/           # crate, rosx-boot-i486, linked at 0x0010_0000
│   └── linker.ld
├── fake-kernel-64/           # crate, x86_64-unknown-none, VA 0xFFFF_8000_0010_0000 AT 0x0010_0000
│   └── linker.ld
└── scripts/                  # gas/ld/objcopy wrappers used by xtask
```

Workspace members: `stage2`, `rosx-boot-common`, `fake-kernel-32`, `fake-kernel-64` (bare-metal,
like the other arch bins — `cargo build --workspace` remains unsupported and each builds with its
explicit `--target`). `stage1` and `stub2.s` are built by the `xtask` boot recipes through
`as`/`ld`/`objcopy` (binutils, already a CI prerequisite via `objdump`), with an existence check
that fails with a clear message when a tool is missing.

## 5. Phases

Each phase is a file `NN-slug.md` in `plans/bootloader/`. **Gate rule: do not start phase N+1
until phase N's automated test passes reliably.** Within a phase, follow the repo's incremental
law: propose each small step, land it, verify, commit (bare imperative subject).

| Phase | File | Supersedes | Deliverable |
|-------|------|-----------|-------------|
| 0 | `00-overview.md`, `00-boot-abi.md` | — | Ground rules, `system::boot_info` treaty |
| 1 | `01-harness.md` | old 0 | `cargo xtask boot-image` / `boot-test`, QEMU feedback loop, exit-code law |
| 2 | `02-stage1-serial.md` | old 1 | 512 B boot sector, COM1 output, `[STAGE1] OK` |
| 3 | `03-stage1-disk-load.md` | old 2 | LBA + CHS-fallback load of stage2, `[STAGE2_16] OK` |
| 4 | `04-a20-protected-mode.md` | old 3a | stage2 crate, A20, flat GDT, 32-bit PM entry |
| 5 | `05-real-mode-thunk.md` | old 3b | `bios_call()` 32→16→32 thunk |
| 6 | `06-memory-map.md` | old 4 | E820/E801/AH=88 detection, normalized `MemoryRegion[]` |
| 7 | `07-fat-storage.md` | old 5a | `BlockDevice` over the thunk + `rxfat`, read `INITRD.BIN` |
| 8 | `08-elf-loader.md` | old 5b | ELF32 loader, `fake-kernel-32` (smoke form) |
| 9 | `09-initrd-crc.md` | old 5c | initrd placement, stage2 CRC32, reservations |
| 10 | `10-handoff-32.md` | old 6 | 32-bit no-MMU handoff, `fake-kernel-32` full form |
| 11 | `11-long-mode-entry.md` | old 7a | CPU guard, page tables, long-mode entry, `fake-kernel-64` (smoke), negative 486 test |
| 12 | `12-handoff-64.md` | old 7b | Higher-half handoff, `fake-kernel-64` full form, completion |

## 6. ISA audit policy

| Artifact | Audit |
|----------|-------|
| `stage1` (raw bin) | `check-isa --stages` deny-list (`xtask/src/check_isa.rs::is_post_486`) |
| `stub2.s` (raw bin) | same deny-list |
| `stage2-32` (raw bin) | same deny-list — must pass **verbatim** |
| `stage2-64` (raw bin) | deny-list **plus** exactly `{cpuid, rdmsr, wrmsr}`, wired as a new audit mode in `check_isa.rs`; mov-to-CR decodes as already-allowed `mov`/`lmsw` |
| `fake-kernel-32` (ELF) | the existing 486 mnemonic allow-list (same bar as kernel + user apps) |
| `fake-kernel-64` (ELF) | not 486-audited (x86_64-only artifact) |

Raw binaries are audited with `objdump -D -b binary -m i386`, as the GRUB stage audit already does.
CI gains a `bootloader` job (build all artifacts, run audits, run `cargo xtask boot-test`) and
`cargo xtask test` runs the boot-test set unless `--skip-integration` is passed.

## 7. QEMU feedback law (measured, this machine)

- Success: guest writes `0x10` to port `0xF4` → QEMU exit code **33**.
- Failure: guest writes `0x01` to port `0xF4` → QEMU exit code **3**.
- **Never `0x00`.** Measured: writing `0x00` exits with code `1`, and a triple fault / guest reset
  under `-no-reboot` also exits with code `1` — the success signal must not alias the crash
  signal. Exit `1` is therefore *defined* as "crashed before reaching a verdict".
- Timeout (default 10 s wall) → failure; any other exit code → failure.
- The harness logs the QEMU version line with every run (upstream behavior around debug-exit and
  boot ROMs has shifted across releases).

## 8. Completion definition of this plan

1. `cargo xtask boot-test` runs every phase checkpoint green in CI, both media (32-bit and
   64-bit), on `-cpu 486` and `-cpu qemu64` respectively, plus the negative CPU-guard test.
2. Every shipped bootloader binary passes its §6 audit; all build surfaces at zero warnings.
3. The existing boot stack is provably untouched: all current unit + integration suites pass
   unmodified.
4. `arch/x86-bootloader/README.md` lets a human rebuild and run every artifact by hand.
5. The `system::boot_info` treaty is frozen (including handoff registers and GDT layout) and the
   follow-on list below is recorded, not silently carried.

## 9. Follow-on plan stubs (explicitly out of scope here)

- **Real-kernel integration.** x86_64: higher-half linker script + `AT()` physical base, verify
  the kernel code-model recipe out of rustc/rust-lld (no `-mkernel` rustflag exists — spike
  first), replace `bootloader_api::entry_point!`/`.bootloader-config`, retire the
  `physical_memory_offset` consumers (`apic.rs`, `main.rs`) and the `clear_nx_bits` NX workaround,
  honor the frozen GDT/`STAR` contract, decide the framebuffer story (the real x86_64 port today
  renders glyphs onto a linear framebuffer handed over by the `bootloader` crate and has **no**
  VGA-text fallback — either stage2 grows a VESA VBE mode-set filling `DisplayKind::LinearFramebuffer`,
  or the port gains VGA text). x86_32: dual-magic entry (EAX `0x2BADB002` → Multiboot path, EAX
  `BOOT_INFO_MAGIC` → `BootInfo` path), Multiboot header kept for coexistence.
- **Carrier swap.** `x86-runner` `BootMedium` for both arches, `-kernel` retirement decision,
  `tests-integration` re-wiring, GRUB ISO + era-media disposition.
- **rom.img as initrd** (decision D5): load `/boot/rom.img` from disk, kernel registers the ROM
  from the initrd, drop the `include_bytes!` coupling and the build-order asserts.
- **Era hardware.** Extend the AGENTS.md hardware checklist to the custom chain (real-drive INT 13
  timing margins, A20 via 8042, E820/E801/AH=88 on period BIOSes, 8250-vs-16550 FIFO tolerance);
  decide whether the custom chain replaces GRUB Legacy on 486 silicon.
