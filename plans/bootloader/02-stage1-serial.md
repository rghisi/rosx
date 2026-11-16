# Phase 2 — Stage 1 Boot Sector & `COM1` Serial Output

Real stage1: a 512-byte MBR-resident boot sector in `.code16`
(`arch/x86-bootloader/stage1/boot.s`) that comes up the serial console and proves it.

## 1. Implementation

- `_start` at `.org`-equivalent `0x7C00` (`ld -Ttext 0x7C00 --oformat binary`); bin layout is
  **code ≤ 446 bytes, zero-filled partition table 446..510, signature `0x55AA` at 510**. The
  size gate is "file is exactly 512 bytes **and** code region ≤ 446" — on the partitioned media
  the table is live (phase 3), and the era scripts' GRUB stage1 proves 446 bytes suffices for
  far more than this (`arch/x86-86box/scripts/build-legacy-hdd.sh`).
- Entry: `cli`; `AX = 0` → `DS = ES = SS = 0`; `SS:SP = 0x0000:0x7C00` (grows down into the
  real-mode window of the ABI reservation map, `00-boot-abi.md` §5); preserve the boot drive in
  `DL` (push it — stage2 needs it); everything stays inside the `0x500–0x7DFF` window — nothing
  above `0x7E00` is touched before stage2 loads.
- 16550 bring-up on `COM1 = 0x3F8`: IER=0 → DLAB=1 → divisor 1 (115200) → 8N1 → FIFO enable +
  reset; every transmit polls `LSR.THRE` (never assume FIFO presence — an 8250 silently ignores
  the FIFO bit and real era UARTs vary).
- `serial_putc` / `serial_puts` / small `put_hex` written inline (this is asm; a tiny macro layer
  in the file, budget-checked).
- Emit `[STAGE1] OK` then halt (`hlt` loop) — disk loading is phase 3.

## 2. Verification

New `boot-test` rows:

1. `stage1-serial` on `boot32.img`, `-cpu 486`: the row runs with a 2 s timeout and asserts
   stdout contains `[STAGE1] OK` regardless of exit code — there is no debug-exit writer yet.
   (First and last exception to the exit-code law; from phase 3 every row judges exit code +
   stdout together.)
2. Static gates enforced by `boot-image` itself: stage1 bin is exactly 512 bytes; code bytes
   beyond 446 are zero; trailing signature present; `objdump -D -b binary -m i386 -M
   i8086,att`-style audit reusing `check-isa --stages` deny-list wiring (raw bin) reports zero
   offenders.

**Pass criteria:** both gates green; phase 1 rows still green (nothing regressed); commit.
