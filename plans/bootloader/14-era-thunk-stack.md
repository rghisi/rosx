# Phase 14 — Era fix: run the BIOS legs on a true real-mode stack

The Phase 13 era evidence run (86Box `b32`: Award 4.51, ASUS PVI-486SP3, i486SX) exposed a
latent thunk defect: INT 13h through the thunk returns garbage geometry on era BIOSes while
SeaBIOS never notices. The root cause is a single unreal-mode pitfall in the leg's stack
setup; this plan fixes it in one instruction and proves it on the era VM.

## 0. Ratified facts (measured 2026-11-17, GDB stub + write-watchpoint)

- **Symptom on era**: `[INT13] 1c 1h 0s` (28 cyl / 1 head / 0 sectors) → `[ERR] disk` →
  BIOS reboot loop. Identical code on QEMU/SeaBIOS: `[INT13] 31c 16h 63s`, boot proceeds.
- **The Award INT 13h handler executes `sti` at entry** (`F000:B2A6`: `xchg bx,bx; sti`).
  The thunk's `cli` is defeated the moment the handler runs; timer IRQs and the C000
  PCI/PnP BIOS ISR chain execute *inside* the call.
- **The leg's stack is not where the design says it is.** `pm16_landing` clears CR0.PE and
  loads `mov sp, RM_STACK_TOP` (16-bit). Clearing PE does not clear ESP's upper bits, so the
  leg runs at linear `0x0007_7A00` — the stage2 stack region — not at `0x7A00`. Measured at
  the watchpoint: INT 13h frames at `0x779Exx`.
- **Why aliasing corrupts the result**: BIOS-era ISRs treat SS:SP as 16-bit words — the
  C000 PnP handler stores SS:SP to `0:0x1488` (`mov ss, word ptr es:[0x1488]`) and restores
  it, and 16-bit saves of an ESP with non-zero upper bits cannot round-trip. A hardware
  write-watchpoint on the handler's saved-AX slot caught `push dx` at `C000:2C54`
  (`ESP=0x779E6`) landing exactly on it. The handler sometimes succeeds internally
  (real `CX=0xDFFF` seen in the frame) while the value the thunk publishes is garbage.
- **SeaBIOS is immune by design**: it never executes `sti` inside INT 13h/15h/10h, so no IRQ
  window exists and the upper-bit aliasing never fires. stage1's classic real-mode INT 13h
  works on era too — the era BIOS was only ever tested with stacks whose ESP fits 16 bits.
- **The window is free**: trampoline occupies `0x600..0x800` (incl. geometry packet `0x580`);
  DAP/data bounce `0x40000`, E820 bounce `0x40000`, BootInfo `0x30000`, payload ≥ `0x100000`,
  stage2 image ≥ `0x7E00`. The leg-stack window `0x800..0x7A00` (grows down from `RM_STACK_TOP`)
  is touched by nothing else.

## 1. The fix

`pm16_landing`, one instruction:

```asm
    mov ss, ax
-   mov sp, {rm_stack}      # 16-bit: upper ESP bits survive clearing CR0.PE
+   mov esp, {rm_stack}     # 386 0x66 form: ESP := 0x00007A00 exactly
```

The `66`-prefixed immediate is 386-era (486 floor untouched). With ESP upper bits zero,
linear == SP for the whole leg: every BIOS ISR's 16-bit SS:SP save/restore round-trips
exactly, and IRQ frames land in the conventional window DOS-era firmware was validated
against. No layout, wire, or contract change: the far-pointer table, `M_*` metadata,
checksums, and the `regs`/`wide` slots stay byte-identical; `restore_frame` keeps restoring
the caller ESP from `{saved_esp}` as before. If the extra byte overflows the code region's
`.skip 0x120` pad, reflow the metadata table one slot (far pointers → `0x128`,
`TRAMPOLINE_STABLE_BYTES` → `0x14C`, mutable scratch `+8`), keeping the 0x200 budget.

## 2. Gates

- `cargo test -p rosx-boot-common`, `cargo xtask boot-test` **3× 24/24** (SeaBIOS must be
  unaffected — the leg semantics are strictly more conventional there),
  `cargo xtask check-isa bootloader`.
- Era truth (the point of the phase): 86Box `b32` boot of `boot32.img` must show real
  geometry on `[INT13]`, then `[DISK]`/`[ELF]`/`[INITRD]`, and the fake kernel's B.3 VGA
  append lines — the full corpus rendered by the era BIOS. Emulator evidence never flips a
  silicon checkbox; the hardware checklist stays open.
- Bonus row: `boot64.img` on the 486SX VM must land on the long-mode guard line, era-clean.

## 3. Era round two — the staging relocation (measured 2026-11-17, INT13-site trace)

The stack fix moved era past the geometry read and exposed the next era truth. Measured at
the thunk's INT13 site (`int 13` at linear `0x666`, decode `linear = cs<<4 + eip`; stage1
never executes there, so the bp sees stage2 traffic only):

- **era rejects the EDD probe**: `AH=41/BX=0x55AA` answers without the marker, so
  `probe_media` falls to the classic path — media is **CHS**, not LBA (the first run's
  AH=42 assumption was wrong; the trace is `AH=08 → AH=41 → AH=08 → AH=02` and silence).
- **the first `AH=02` read storms**: request `AX=0201 BX=0 CX=0001 DX=080 ES=4000` — the
  bounce segment — and control never returns past the handler; the Award RAM IDE driver
  (`CS=0xB400`, code in the `0xA0000-0xE8000` hole E820 never reports) enters its internal
  reset-retry (`AX=1201`) recursively, descending ~230 B per retry through the leg-stack
  window until ESP wraps below zero and the box dies.
- **the request is byte-identical to stage1's proven era read except for ES**: stage1
  failed the same EDD probe and loaded stage2 with the identical `AH=02`, ES segments
  `0x90`/`0x7E0`-`0x1690`. The geometry/packet path (transfer at `0x580`) is era-clean.
- **era's own E820 does not reserve the landmine**: collected at `0x40000` (24-byte entries
  despite `ECX=20` — era ignores the requested size), the map marks `0x0-0x9FC00` usable,
  `0xE8000+` reserved — the RAM driver's scratch window is undocumented. `0x40000` is a
  legal RAM address for an OS; the era driver simply owns it under the table.

**Fix**: stage everything below the driver's territory. The strip between the stage2 top
(`0x7E00` + 62.6 KiB = `0x17A10`) and the BootInfo/map home (`0x30000`) fits the 32 KiB
bounce and 64 KiB heap exactly:

- `BOUNCE_ADDRESS 0x40000 → 0x18000`, `BOUNCE_SELECTOR 0x4000 → 0x1800` — `HEAP_ADDRESS`
  follows to `0x20000`, heap ends at exactly `0x30000`, adjacent to the map home.
- the E820 bounce moves to the same region (per-entry copy-out is immediate — the two
  bounces never share the window in time); era's own E820 then lands at `0x18000`.
- `STAGE2_MAX_BIN_BYTES 160 KiB → 66048` (`0x7E00..0x18000` budget) so the existing
  build-time guard protects the new neighbour: 32-bit stage2 is 60312 B, 64-bit 62608 B.
- the reservation model (`memnorm.rs`, mirrored by the boot-test golden) frees
  `0x31000..0x7C000` as Usable and drops the `0x40000..0x7C000` bootloader row;
  `0x0800..0x7C00` already reserves the leg-stack window, `0x7E00..0x30000` already
  reserves image+bounce+heap as one row.

Era acceptance: media=CHS on `[DISK]`, then `[ELF]`/`[INITRD]` and the fake kernel's B.3
VGA append lines. QEMU keeps all its gates: SeaBIOS transfers to any conventional address.

## 4. Commit sequence (each gate green before the next)

1. `Plan the true real-mode thunk stack for era BIOSes` (this file)
2. `Run BIOS thunk legs on a true real-mode stack` (the fix)
3. `Plan the staging relocation for the era RAM-driver window` (§3)
4. `Stage disk and E820 transfers below the era RAM-driver window` (the relocation)
5. `Record the era proof of the relocated staging` (README era note + run directory)
