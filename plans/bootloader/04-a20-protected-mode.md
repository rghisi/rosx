# Phase 4 — A20, Flat GDT, 32-bit Protected Mode (i486)

Stage2 graduates from the asm stub to its crate. This phase lands the Rust stage2 image entering
32-bit flat protected mode cleanly on `-cpu 486` — still no BIOS calls beyond A20, no thunk yet.

## 1. The stage2 crate

- `arch/x86-bootloader/stage2/`, workspace member, `#![no_std]`, builds on a new
  `rosx-boot-i486.json` target spec copying `arch/x86_32/rosx-i486.json`'s pinned feature string
  (`-mmx,-sse,-sse2,-cmov,-fxsr,-cx8`) — the compiler *cannot* emit i686+ instructions; the
  `check-isa`-style audits are the second gate. Never export `RUSTFLAGS` when building it (the
  `.cargo/config.toml` rustflags carry the linker script; same trap as the x86_32 user ELFs).
- `linker.ld`: `.text.entry` first at `0x7E00`, then text/rodata/data, `.bss` last with exported
  `_bss_start`/`_bss_end`; entry stub in `global_asm!`: zero `.bss` (raw-binary load means no
  loader zeroes it — a requirement this plan must satisfy itself), set `ESP` to the PM stack top
  **`0x0007_F000`** (reserved window `0x7C000–0x7FFFF` of the ABI table), `call stage2_main`.
- Panic handler: print panic info over COM1, write `0x01` to `0xF4` (exit 3), halt.
- Feature `stage-64` (default off) selects the long-mode module in later phases; the `stage-32`
  build never sees that code, which is what keeps its deny-list audit verbatim-clean.
- Size gate: the flat bin ≤ 160 KiB, enforced in `boot-image` (the gap and the 0x7E00–0x2FFFF
  reservation both depend on it).
- xtask: `boot-image` now packs the real stage2 (objcopy'd) behind the `ROS2` header; the
  `stage2-16` checkpoint row flips to `[STAGE2_32]` names.

## 2. A20 (with verification, not hope)

Order: `INT 15h, AX=2401h` → port `0x92` bit 1 (read-modify-write, never blindly `in`/`or` —
bit 0 is the fast-A20 gate's sibling and the keyboard controller line matters) → 8042 (`0xD1`
command, `0x60` data with bit 1 set, output-buffer timeouts guarded). **Verify** the result by
the classic wraparound probe in 16-bit: write a sentinel at `0x0000:0x0500`, compare against
`0xFFFF:0x0510`'s alias after clearing, restore originals; failure prints `[ERR] a20` and exits
3 (a silent A20 no-op would corrupt everything above 1 MiB later).

## 3. GDT + protected mode

- Flat GDT per the ABI table (`00-boot-abi.md` §4) **including the thunk selectors** `0x18`
  (16-bit code) / `0x20` (16-bit data) now, so phase 5 never reshuffles the table:
  `0x08` code D=1 / `0x10` data, both 4 GiB flat; `lgdt`; set `CR0.PE` (`mov cr0` — `mov` is
  deny-list-clean); `ljmp $0x08`; reload `DS/ES/FS/GS/SS = 0x10`; jump to the Rust side.
- From 32-bit: re-assert A20-visible behavior by re-running the wraparound check with 32-bit
  addressing, print `[STAGE2_32] A20=OK`.

## 4. Verification

1. `stage2-pm-486` on `boot32.img`, `-cpu 486`: stdout `[STAGE1] OK` + `[STAGE2_32] A20=OK`,
   exit **33**. No `#UD`, no triple fault (exit 1 is loudly distinct).
2. Deny-list audit (`check-isa --stages` machinery, raw bin) on the stage2-32 flat binary —
   zero offenders; also on stage1 after its growth.
3. Size gate row: `boot-image` refuses to pack a > 160 KiB stage2.
4. Host unit tests in the stage2 crate for the pure parts (A20 probe sequencing stays asm-gated;
   GDT descriptor encoding is pure — table tests like `check_isa`'s style).

**Pass criteria:** rows 1–3 green; every prior row green; commit.
