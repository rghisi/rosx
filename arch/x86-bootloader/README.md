# arch/x86-bootloader — custom x86 BIOS bootloader

The ROSX-native boot chain for both x86 ports: a BIOS MBR boot sector
(`stage1`) that loads a protected-mode loader (`stage2`) from raw LBAs, a
FAT16 boot volume carrying the kernel ELF and an initrd, and a frozen
handoff contract (`plans/bootloader/00-boot-abi.md`) that boots an ELF32
kernel in 32-bit protected mode or an ELF64 kernel in long mode through the
higher half. Blueprint: `plans/bootloader/` (00-overview + phases 01–12);
the checkpoint suite and media packer live in `xtask` (`boot_image.rs`,
`boot_test.rs`, `check_isa.rs`).

**Positioning:** x86_32 boots production through this chain — stage1 and
stage2-32 on the unified FAT boot disk (`plans/x86_32-carrier-swap.md`). The
x86_64 production boot stack is untouched — it still boots via the
`bootloader` crate. The chain is proven in QEMU (the `bootloader` CI job:
`cargo xtask boot-test` + `cargo xtask check-isa bootloader`, including the
`real-kernel-boot32` checkpoint that boots the real kernel off the built
disk); x86_64 real-kernel integration, rom.img-as-initrd and era-hardware
verification are the recorded follow-on stubs (`00-overview.md` §9). Nothing
goes green on hardware without the AGENTS.md hardware evidence rules.

## Layout

```
common/        pure-logic library crate: crc32, memnorm (memory-map
               normalization), pagetable (identity + higher-half maps), gdt,
               elf32/elf64 planners, bootinfo assembly, BIOS data, INT 13h
               packet layout, debugport/pio
stage1/        boot.s — MBR boot sector: loads the stage2 image from LBA 8 to
               physical 0x7E00 (INT 13h LBA extensions, CHS fallback), jumps
               in real mode
stage2/        the loader (one crate, two flavors): default = Flat32,
               --features stage-64 = long-mode chain. Entry stub: A20 via the
               fast gate with an 8042 fallback, then protected mode. Main:
               memory map detection (INT 15h E820/AH=88/E801 via a real-mode
               thunk), FAT16 volume reader, ELF loader, BootInfo writer, handoff
fake-kernel-32/ ELF32 smoke kernel: validates the whole 32-bit handoff
               contract (magic, header, map walk, initrd crc32), exits 33
fake-kernel-64/ ELF64 smoke kernel, linked at 0xFFFF800000100000
               (physical 0x100000): validates the 64-bit contract — RAX magic,
               higher-half RDI/RSP, runtime RIP in the higher half, dual-map
               agreement (window vs identity alias), map walk, initrd crc32
scripts/       harness_stub.s — the harness self-test stubs (pass/fail/crash/hang)
rosx-boot-i486.json   custom target: 486 floor, features pin
               -mmx,-sse,-sse2,-cmov,-fxsr,-cx8 (same floor as the x86_32 port)
rosx-boot-x86_64.json custom target: kernel code model, -mmx,-sse,+soft-float
```

## Media layout

| Region | Content |
|---|---|
| LBA 0 | MBR: stage1 boot code + one bootable FAT16 slice (type 0x06) at LBA 2048 |
| LBA 8 | stage2 image: `[512-byte "ROS2" header][flat bin]`, budget 160 KiB (physical 0x7E00..0x2FFFF) |
| LBA 2048.. | FAT16 volume `ROSXBOOT`: `KERNEL.ELF`, `INITRD.BIN` (64 KiB seed), `PROBE.TXT` |
| RAM 0x30000..0x30FFF | `BootInfo` (top of the window, 16-byte aligned) + normalized memory-map array; the payload span (kernel, initrd, page tables) is published as `KernelAndModules`, the window itself as `Reserved` |

The floppy image (`boot-image floppy`) carries stage1 as its boot sector and
the stage2 blob at the same LBA 8, exercising the CHS fallback path.

## Handoff (frozen contract, `00-boot-abi.md` §3)

| | 32-bit (`ELF32`/`EM_386`) | 64-bit (`ELF64`/`EM_X86_64`) |
|---|---|---|
| CPU | protected mode, paging off, `CLI`, A20 on, flat GDT | long mode, `CR0.PG=CR4.PAE=EFER.LME=1`, `CLI`, GDT |
| Magic | `EAX = BOOT_INFO_MAGIC` | `RAX = BOOT_INFO_MAGIC` |
| BootInfo | `EBX` = physical pointer | `RDI` = higher-half pointer |
| Stack | `ESP` 16-aligned physical | `RSP` 16-aligned higher-half |
| Transfer | `jmp` entry | `jmp` entry |
| Paging | none (identity by construction) | identity `0..4 GiB` **and** the `0xFFFF800000000000` window, both live at entry |

The `stage-64` flavor gates itself with a CPUID extended-leaf probe: a 486
(or `-lm`-stripped) CPU gets `[ERR] CPU does not support x86_64 Long Mode`
and a clean debug-exit failure — never a `#UD`. All 586-era mechanism
(`cpuid`, `rdmsr`, `wrmsr`, long-mode entry) lives in the dedicated
`.longmode.text` section; the rest of the stage2-64 binary stays 486-clean.

## Verdict law

Every guest verdict rides `isa-debug-exit,iobase=0xf4`: success `0x10` →
shell exit **33**, guest-reported failure `0x01` → exit **3**, no write →
QEMU must be timed out. The QEMU debug port (`-debugcon`, port 0xE9) carries
the stage banners (`[STAGE1] OK`, `[MEM88]`, `[DISK]`, `[ELF]`, `[INITRD]`,
`[PAGETABLE]`, `[MEM]`, `[STAGE2_32]/[STAGE2_64]`) and the fake kernels'
validation lines.

That port is the **verdict stream** — the only channel any checkpoint reads. In parallel the
same text rides the **human stream**, the BIOS VGA text console: stage1 teletypes every
character through `INT 10h, AH=0Eh` (page 0); stage2 broadcasts through a `BroadcastConsole`
(debug port + teletype over the already-installed BIOS thunk); the fake kernels — post-handoff,
where the 16-bit ROM is unreachable — append directly to page-0 `0xB8000` at the live CRTC
cursor (`0x3D4/0x3D5` indices `0x0E`/`0x0F`; SeaBIOS's teletype leaves the BDA pair stale, so
the CRTC is read and the BDA pair re-published) and clamp at the bottom row. Stage1 and stage2
own zero display state — the BIOS owns cursor, glyphs and overflow
(`plans/bootloader/13-console.md`).

## Build and run

Host tools: `qemu-system-i386`, `qemu-system-x86_64`, mtools
(`mformat`/`mcopy`/`mdir`/`mdel`), binutils (`as`/`ld`/`objdump`/`objcopy`).

```bash
cargo xtask boot-image 32       # target/boot32.img  (stage2 Flat32 + fake-kernel-32)
cargo xtask boot-image 64       # target/boot64.img  (stage2 stage-64 + fake-kernel-64)
cargo xtask boot-image floppy   # target/floppy.img (CHS path)
cargo xtask boot-test           # 24 checkpoints, both binaries + negative guards
cargo xtask check-isa bootloader  # 486 audit: fake-kernel-32 + stage2 (.text under
                                  # the i386 deny-list; .longmode.text under the
                                  # x86-64 decoder admitting exactly cpuid/rdmsr/wrmsr)
cargo test -p rosx-boot-common  # the pure logic (normalizer, planners, page tables)
```

Boot a built image by hand (64-bit chain, expect exit 33):

```bash
qemu-system-x86_64 -cpu qemu64 -m 128M -drive format=raw,file=target/boot64.img \
  -debugcon file:/tmp/rosx64.debugcon -display none -monitor none \
  -device isa-debug-exit,iobase=0xf4,iosize=0x04 -no-reboot; echo $?   # 33
```

Swap `-cpu 486 -m 32M` + `target/boot32.img` + `qemu-system-i386` for the
32-bit chain, or `-cpu qemu64,-lm` + `target/boot64.img` for the guard row
(expect `3`). The checkpoint suite leaves its images (including the negative
cases: patched ELFs, scribbled FAT root, corrupt stage2 header, oversized /
absent initrd) under `target/boot-test/`; boot any of them with the same
command line.

## Checkpoints (24)

| Row | Machine | Proves |
|---|---|---|
| `harness-pass/fail/crash/hang` | 486 | the harness itself: all four verdict classes classify |
| `stage2-pm-486` | 486 32M | stage1→stage2 on a 486: A20, thunk, INT 15h, geometry |
| `stage2-hdd-lba-x86_64` | qemu64 128M | the same chain on the 64-bit host binary |
| `stage2-floppy-chs` | 486 floppy | CHS fallback media |
| `stage2-map-512m` | 486 512M | map normalization above 512 MiB (`[MEM] … OK`) |
| `handoff32`, `handoff32-512m` | 486 | full 32-bit handoff: fake kernel validates and exits 33 |
| `handoff32-no-initrd` | 486 | absent initrd: `addr=0 size=0`, empty-payload crc32 |
| `handoff64`, `handoff64-512m` | qemu64 | full 64-bit handoff: RAX/RDI/RSP/entry-RIP, dual map, initrd crc32 through the window |
| `initrd-too-big` | 486 8M | placement refusal (`[ERR] mem`) before the kernel boots |
| `fat-boot64-media` | qemu64 | the FAT16 engine reads the 64-bit volume |
| `fat-corrupt` | 486 | scribbled root directory fails as `[FAT-ERR]` |
| `elf32-load` | 486 | ELF32 banner: entry/phys placement |
| `elf32-class64`, `elf32-zero-paddr` | 486 | ELF32 loader refuses class-2 and zero-paddr images |
| `elf64-class32` | qemu64 | ELF64 loader refuses the class-1 kernel (`[ERR] elf class`) |
| `lm-smoke` | qemu64 | long-mode entry: ELF64 load, page tables, kernel banner |
| `lm-guard-486`, `lm-guard-nolm` | 486 / `-lm` | the CPU guard exits 3, never a `#UD` |
| `stage2-corrupt-header` | 486 | a bogus ROS2 magic hangs stage1 (timed-out verdict) |

The generic verifiers read `[MEM] entries=` and `[INITRD] addr=` off the
debug-port stream: the published map must re-derive from the host-computed seed,
and the initrd crc32 must match three ways (stage2 banner, fake-kernel
recompute, host seed) — so a passing row proves the guest read real bytes.

## Era context

The 486 baseline rule of the x86_32 port applies verbatim here: everything
in `.text` (both flavors, both fake-kernel-32 and the whole stage1/stage2
chain) must execute on a 486DX — no `cpuid`, no `cmov`, no MMX/SSE, no FPU
state saved across the switch. `cargo xtask check-isa bootloader` audits the
shipped binaries against that floor and every 32-bit checkpoint boots under
QEMU `-cpu 486`; the 64-bit flavor carries the 586-era exception as a
runtime probed path, not a compile-time assumption, so the same era silicon
fails loudly instead of faulting. The boot console rides port 0xE9 for the
same era reason: QEMU traps it as a debug console, while on silicon it is an
unclaimed ISA port whose writes are architecturally harmless no-ops — the
banners evaporate and the boot proceeds.

The human stream is era-proven on the 86Box `b32` VM ([SiS 496] ASUS
PVI-486SP3, i486SX, Award 4.51, S3 Trio64): a `screen_text` reading records
stage1's banner, stage2's thunk-teletype banners and the honest `[ERR]`
verdict rendered by the era BIOS itself (run protocol and recorded run
directories: `arch/x86-86box/`; emulator evidence never marks hardware items
green). Known era limitation, recorded in that run's `facts.md`: INT 13h
issued through the unreal-mode thunk returns garbage geometry on this Award
BIOS — the handler executes `sti` at entry, so IRQs land on the thunk's
return slots — while SeaBIOS keeps interrupts masked inside INT 13h and every
checkpoint passes unchanged. Era disk boot through the thunk stays open
bring-up work, independent of the console.
