# Phase 3 — Stage 1 Disk Loading (LBA + CHS Fallback) → Stage 2 Entry

Stage1 grows from "print and halt" to "load the stage2 image and jump" — on both the era-hard-disk
path (INT 13h LBA extensions) and the pre-1995 fallback (CHS), matching the two media the harness
packs.

## 1. Implementation (stage1 additions, still inside the 446-byte budget)

1. **LBA probe:** `INT 13h, AH=41h, BX=55AAh` on the boot drive; support iff CF clear **and**
   `AH == 0x01` and `BX == 0xAA55`.
2. **LBA read:** `AH=42h` with a Disk Address Packet on the stack (packet at a fixed offset in
   stage1's own sector, count from the stage2 header). Read the header sector (LBA 8) first,
   validate `magic == 'ROS2'` and `total_bytes ≤ 160 KiB + 512`, then read the remaining sectors
   of the image into `0x0000:0x7E00`.
3. **CHS fallback** (LBA unsupported — super-floppy media): geometry via `INT 13h, AH=08h`;
   LBA→CHS conversion (sectors-per-track from the result table, heads ≤ 256); sector-at-a-time
   `AH=02h` reads with retry (3 attempts, `AH=00h` controller reset between failures) into
   `0x0000:0x7E00`.
4. Transfer errors print `[ERR] disk` + the failing LBA in hex and halt (no debug-exit exists
   yet at this phase — the harness row judges stdout + timeout).
5. **Jump:** `ljmp $0x0000, $0x7E00`, `DL` (boot drive) preserved.
6. Budget discipline: if the 446-byte gate breaks, the retry/reset logic in the CHS path is the
   designed sacrifice (documented in the file), not the LBA path.

## 2. Stage2 entry stub

`arch/x86-bootloader/stage1/stub2.s` — a minimal `.code16` flat binary at `0x7E00` (not yet the
Rust crate): reuse the 16550 init + `serial_puts`, print `[STAGE2_16] OK`, write `0x10` to port
`0xF4`, `hlt`. It is replaced wholesale by the stage2 crate in phase 4; until then it carries the
`[STAGE2_16]` checkpoint name.

## 3. Verification (new `boot-test` rows)

1. `stage2-hdd-lba`: `boot32.img` (MBR partition media, `-cpu 486`) → stdout contains
   `[STAGE1] OK` then `[STAGE2_16] OK`, exit **33**.
2. `stage2-floppy-chs`: `floppy.img` super-floppy (`-drive format=raw,if=floppy,...`) → same
   stdout pair, exit **33**. This row is the CHS path's only automated coverage — its pass is
   non-negotiable for the 486 claim.
3. Corrupt-header negative row: an image whose stage2 header magic is garbage → stdout contains
   `[ERR] disk`, **not** `[STAGE2_16]`, timeout/crash verdict accepted as the failure shape.
4. Static: stage1 code still ≤ 446 bytes; deny-list audit green on both bins.

**Pass criteria:** all four rows green; prior rows green; commit.
