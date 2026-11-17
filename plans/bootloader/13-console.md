# Phase 13 — Console rework: debug-port truth, then BIOS VGA voice (two phases)

Split the bootloader's single console into the two channels the platform actually has:
the **QEMU debug port (0xE9)** as the machine-readable verdict stream (Phase A, which
retires the 16550 serial stack entirely), and the **BIOS-owned VGA text console** as the
human-visible stream (Phase B, zero display state and zero scrolling in our code).
The verdict law, the wire, and the frozen ABI do not move.

## 0. Ratified facts (verified 2026-11-16, no repo changes)

- **The OS side already runs this doctrine.** Both ports write debug output to port `0xE9`
  (`arch/x86_32/src/main.rs`, `arch/x86_64/src/main.rs` via `x86_generic::debug_console`),
  and `tests-integration` boots with `-serial none` capturing QEMU stdout — where the
  default `-debugcon` lands. The bootloader adopting the same split is alignment, not novelty.
- **AGENTS hardware law already sanctions both ends**: "VGA is the only output; writes to
  the unclaimed debug port 0xE9 must stay harmless" — unclaimed ISA port writes are
  architecturally harmless no-ops, so the emulator channel evaporates on silicon.
- **Measured**: a real-mode stub emitting `outb $0xE9` loops under `qemu-system-i386
  -cpu 486 -debugcon file:...` captured byte-exact output; `-serial file:` coexisted;
  the `isa-debug-exit` verdict (0x10→33 / 0x01→3 / hang) was unaffected.
- **`BootInfo.display` already declares `VgaText, 0xB8000, 80×25, pitch 160`** and the
  treaty test pins it (`assemble_declares_vga_text_per_the_abi_table`). The map never marks
  `0xA0000..0xF0000` usable, so the screen is protected by construction.
- Current corpus: the 64-bit boot prints 21 stage lines + 3 fake-kernel lines = **24 rows**
  on an 80×25 screen; the 32-bit boot, 22. One row of slack at these geometries.
- `common::serial` is more than a transport: `write_hex`/`write_dec`/`write_line` and the
  `core::fmt::Write` impl (panic path) live there and are the shared formatter surface.

## 1. Phase A — migrate every producer to 0xE9, delete the serial stack

### A.1 `common::debugport`

`DebugConsole`: the exact surface today's `Serial` offers — `write_str`, `write_line`,
`write_hex`, `write_dec`, plus the `fmt::Write` impl — with the byte path a single
`out 0xE9`. No init (that is the point). The hex/decimal renderers and `HEX_DIGITS`
*move* out of `serial.rs` rather than duplicating; unit tests move with them.

### A.2 The atomic flip (one commit — and it must be one commit)

The checkpoint corpus is a collaboration of producers (`[STAGE1] OK` is stage1 asm,
`[FAT-ERR]` is stage2, `initrd_crc32:` is the fake kernel, `BOOTLOADER-HARNESS OK` is
the stub), so per-producer flips can only land together with the capture switch:

- `stage1/boot.s`: delete the COM1 init block; `putc`'s TX-poll loop becomes one `outb 0xE9`.
- `stage2`: every `Serial::COM1` (main, memory, fat, elf32, longmode, heap, panic handler)
  becomes `DebugConsole` — type substitution only, all strings and call sites invariant.
- `fake-kernel-32/64`: same substitution, including panic handlers.
- `scripts/harness_stub.s`: same substitution as stage1.
- `xtask/boot_test.rs`: `boot()` swaps `-serial file:` for `-debugcon file:` (same
  pre-delete-then-capture model); the fields rename `serial_contains/serial_absent` →
  `console_contains/console_absent`, `dump_serial` → `dump_console`. Purely mechanical:
  all 24 checkpoints keep their exact strings, and the three-way `[INITRD]` crc audit and
  `[MEM] … OK` verifiers read the same bytes from the new file.

Gate: `cargo test -p rosx-boot-common`, `boot-test` **3× consecutive 24/24**, `check-isa bootloader`.

### A.3 Deletion

Remove `common::serial` and `COM1_PORT` (A.2 leaves no consumer — repo law: no dead
surface). Re-measure the stage1 footprint: code+data ends at byte 427 of 446 today; the
COM1 init (~35 B) and TX poll loop (~20 B) come back, restoring roughly 70 bytes of MBR
headroom. Update `arch/x86-bootloader/README.md` (serial console section becomes
debug-port; hand-boot lines use `-debugcon file:`; the era note gains the
"0xE9 is harmless on silicon" rationale). Add the `00-overview.md` §9 stub line
acknowledging this rework.

Gate: unit suite, `boot-test` 3×, `check-isa`.

## 2. Phase B — VGA text output, BIOS as much as possible, no scrolling in our code

Display-state doctrine: **stage1 and stage2 own zero display state.** The BIOS owns cursor,
glyphs, page and — if the budget were ever exceeded — overflow. Our code never scrolls,
tracks rows, or touches attribute memory.

### B.1 stage1 — real mode, direct BIOS

`putc` additionally issues `INT 10h, AH=0Eh` teletype (AL=char, BH=page 0): ~6–8 bytes,
affordable post-A.3. Era-native; nothing else needed.

### B.2 stage2 — protected mode via the existing thunk

`BiosTeletype`: `write_str` through the **already-installed BIOS thunk** (`Regs16`,
`INT 10h AH=0Eh`) — the same real-mode trampoline E820 and INT 13h geometry ride. Broadcast
console (constructed in stage2 `main`, lives in stage2 because the thunk type does):
`DebugConsole + BiosTeletype`, formatting reused from `common`. All stage2 prints occur
with paging off (every banner precedes `enter_long_mode`) — verified against `main.rs`
flow, so VRAM access and BIOS real-mode both just work.

Known, documented gap: the two failure lines before thunk install (`[ERR] a20`,
`[ERR] thunk`) are debug-port-only. Not a loss: the debug port is the authoritative
stream and stays so.

Spike option (may stay rejected): `AH=13h/AL=01h` string output to cut thunk round-trips
(≈1 KB of boot text ≈ a few hundred switches, milliseconds; acceptable). Baseline is
per-char teletype — the most universally era-present service.

### B.3 fake kernels — the documented no-BIOS exception

Post-handoff the 16-bit ROM is unreachable (and the fake kernels have no thunk). They
append via **direct VRAM continuation at the BIOS data area cursor**: read row `0x0450`,
col `0x0451`, page `0x0400` (identity-mapped in both flavors), write cells at
`0xB8000` attribute `0x0F`, advance BDA, update the CRTC cursor (`0x3D4/0x3D5`). Gated on
`BootInfo.display.kind == VgaText` (honor the treaty; stay silent otherwise).

No scrolling, by decision: at the bottom row the writer clamps (rewrites the last row).
The budget makes that unreachable at shipped geometries (24 of 25 rows), and overflow
degradation is bounded and visible. Pure logic (cursor math on a `&mut [u16]` buffer,
memnorm-style unit tests) with a thin device shim.

### B.4 Verification stays honest

The debug port remains the only verdict stream; **no boot-test checkpoint asserts VGA
text** (screen capture would mean `screendump` PPM + font matching — rejected as harness
weight). VGA evidence is: dev-loop `qemu -display gtk` eyeball; era truth via the 86Box
MCP `screen_text` (built for text-mode reads), recorded locally, per the 86Box README
positioning (emulator evidence never marks hardware items green).

### B.5 Gates per commit

`boot-test` must stay **3× green on every B commit** (VGA is strictly additive — the debug
stream must be byte-identical to A.3), plus `check-isa bootloader` (stage1 budget), unit
suites, and one recorded 86Box `screen_text` reading of the full boot corpus.

## 3. Commit sequence (each gate green before the next)

1. `Add the QEMU debug-port console to the bootloader common crate` (A.1)
2. `Move the bootloader verdict stream from the serial port to the debug port` (A.2 — atomic)
3. `Retire the 16550 serial stack from the bootloader` (A.3)
4. `Print stage1 on the VGA console through the BIOS teletype` (B.1)
5. `Echo stage2 banners on the VGA console through the BIOS thunk` (B.2)
6. `Append fake-kernel lines to the VGA console at the BIOS cursor` (B.3)
7. `Document the dual-channel boot console` (README + overview §9)

## 4. Decision log

- **Debug port over serial for tests**: zero-init, ~70 MBR bytes back, matches the OS-side
  doctrine, harmless on silicon; the checkpoint corpus changes capture pipe, never strings.
- **Atomic A.2**: the corpus has four producers; any partial flip reddens checkpoints.
- **BIOS-first, direct-VRAM-exception only where BIOS cannot exist** (post-handoff), and
  even then continuing the BIOS's own cursor through the BDA.
- **No scrolling implemented anywhere**: budget (≤25 rows at shipped configs) plus BIOS
  teletype overflow as the safety net plus a clamped bottom row for the fake kernels.
- **Debug port stays the sole verdict stream**: screen-capture OCR in CI was weighed and
  rejected; 86Box `screen_text` is the era-accurate reader for local evidence.
