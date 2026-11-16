# Phase 9 — Initrd Placement + Stage-2 CRC32

The payload channel gets its final shape and its integrity anchor: initrd lands page-aligned
right after the kernel, stage2 CRCs it, and the number is cross-checked against the host.

## 1. Implementation

- After the phase-8 kernel placement: `INITRD.BIN` is streamed from `rxfat` directly to
  **`kernel_phys_end` (already 4 KiB-aligned)**, replacing the phase-7 staging stopgap at
  `0x100000`; `initrd_phys_end` aligned up to 4 KiB; refuse (`[ERR] mem` exit 3) if the span
  would run past detected usable memory (the phase-6 map is consulted — stage2 never writes
  where the BIOS said it can't).
- **CRC32** (IEEE 802.3, reflected, table-less bitwise loop from `rosx-boot-common` — 8 KiB
  tables would be fine but the loop is, matches the era's byte-count, and keeps the shared crate
  dependency-light) computed over the **in-memory** bytes, so it attests what the machine
  actually holds, not what the file claimed.
- Print `[INITRD] addr=0x… size=… crc32=0x…`.
- Memory-map extension: `[kernel_phys_start .. initrd_phys_end]` carved as
  `MemoryKind::KernelAndModules` through the phase-6 normalizer's carve-out parameter (one call
  site, no normalizer special cases).
- `boot-test` gains the expected-CRC plumbing for the *placed* buffer: the host seed already
  determines the bytes, so the assertion is three-way: expected == stage2 log == (phase 10 on)
  kernel log.

## 2. Verification

1. `initrd-crc` on `boot32.img` (`-m 32M`, `-cpu 486`): `[INITRD]` line present; harness
   recomputes CRC32 from the recorded seed and asserts equality with the stage2 log; `[FK32]
   alive` follows; exit 33.
2. `initrd-crc-512m`: same row at `-m 512M` — placement math must respect the bigger map.
3. `initrd-too-big`: packager variant with an initrd that would overrun the `-m 32M` map →
   `[ERR] mem` + exit 3 (the refusal is a feature, tested).
4. Host unit tests: CRC32 known-vectors (`""`, `"123456789"` → `0xCBF43926`) in
   `rosx-boot-common`; placement arithmetic (alignment, refusal bound) as pure-module tests.
5. All prior rows + audits green.

**Pass criteria:** rows 1–5 green; commit.
