# Phase 5 — The 32-to-16-bit Real-Mode Thunk (`bios_call`)

The single most delicate component of stage2: every BIOS service stage2 needs (disk geometry
re-queries, memory detection, and nothing else in this plan) runs through one thunk that drops
from 32-bit protected mode to real mode, executes one `INT xx`, and returns. Everything after
this phase treats it as boring.

## 1. Design

- **Trampoline:** a ~200-byte position-fixed `.code16` blob copied once to **`0x00600`** at
  stage2 init (reservation table: `0x00600–0x007FF`), contents verified after copy
  (checksum compare) before first use.
- **Real-mode stack:** `SS:SP = 0x0000:0x7A00` inside the designated `0x00800–0x07BFF` window,
  far below stage1's own `0x7C00` — BIOS services get a real, low stack, and the IVT at
  `0x0000–0x003FF` is never in the growth path.
- **Sequence** (asm, in `stage2/src/thunk.rs` via `global_asm!`, Rust-facing as
  `pub fn bios_call(int_no: u8, regs: &mut Regs16)`):
  1. Save 32-bit `ESP` and `EFLAGS` to a static slot; disable paging is a no-op here (never on),
     but the sequence is written to tolerate CR0.PG=1 callers by clearing it before PE-clearing.
  2. `ljmp` to the `0x18` 16-bit code selector (self-relative within the trampoline's landing
     stub), load `16-bit data selector 0x20` into `DS/ES/FS/GS/SS`, set `SP` to the real-mode
     stack.
  3. Clear `CR0.PE` (`mov cr0`), `ljmp` to `0x0000:trampoline+off` to flush the prefetch queue
     in true real mode.
  4. Push the caller's `Regs16` values, `INT int_no`, pop results back to the memory block
     (the marshaling lives in low memory, not on the precious real-mode stack beyond what the
     BIOS itself needs).
  5. Set `CR0.PE` with the **32-bit data selectors already re-loadded via a short
     16-bit-protected-mode hop** (`0x18` code with PE=1 first, load `0x10`, then the 32-bit
     `ljmp`), restore `ESP`, `POPF` the saved flags. The flag/segment restore order is pinned by
     comment-free construction: `PUSH`-saved flags are restored last, after all selector loads,
     so no restored flag (e.g. IF) can act mid-sequence. IF never becomes 1 during any thunk.
- **Safety properties stated for audit:** one outstanding `bios_call` at a time (stage2 is
  single-threaded, no interrupts); the caller's segment state is defined by the ABI table on
  return; `0x00600` blob is never rewritten after init.

## 2. Verification

1. `thunk-basic` on `boot32.img`, `-cpu 486`: from Rust, call `INT 15h, AH=88h` (contiguous KiB
   above 1 MiB): expect the plausible `-m 32M` answer (`[MEM88] 31744 KiB ± BIOS slack` printed);
   then re-verify the wraparound A20 probe still passes after returning (thunk must not have
   disturbed memory or selectors); then re-run `INT 13h, AH=08h` on the boot drive and print
   geometry — proof the DAP-era service set works through the thunk too. Print
   `[STAGE2_32] A20=OK THUNK=OK`, exit 33.
2. `thunk-repeat`: call `AH=88h` twenty times in a loop — flakiness in this sequence is the
   classic failure mode, so repetition is the test.
3. Prior rows green; deny-list audits green (trampoline blob included in the stage2 flat bin —
   the `objdump -b binary` sweep covers its bytes, and it must stay 386-clean).

**Pass criteria:** both rows green ≥ 3 consecutive runs (flake bar raised for this phase); commit.
