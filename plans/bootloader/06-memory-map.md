# Phase 6 — BIOS Memory Detection & Normalization

The machine's memory story becomes real: cascade the era's three interfaces, normalize into the
ABI's `MemoryRegion[]`, and machine-check the result — the gate is arithmetic, not vibes.

## 1. Detection (all through the phase-5 thunk)

1. `INT 15h, EAX=E820h` with `EDX = 534D4150h` ("SMAP"), continuation in `EBX`, raw entries into
   the bounce window at `0x0004_0000` (buffer ≥ 64 × 24 B + slack). Loop until `EBX == 0` or
   CF set; accept variable entry sizes (`≥ 20`), re-validate the SMAP signature on every entry
   after the first (era BIOSes mutate it on error returns).
2. Fallback `INT 15h, AX=E801h` (memory 1–16 MiB in `AX` KiB — with the documented BIOS quirk
   that some return 64 KiB blocks; detect the `AX == 0x1C20` sentinel pattern and re-interpret;
   above-16-MiB in `CX` KiB).
3. Fallback `INT 15h, AH=88h` (capped 64 MiB), synthesizing `[0,1M) usable`, `[1M, 1M+KiB)`.
   QEMU always answers E820 — the two fallbacks are covered by **host unit tests** of the
   synthesizer/normalizer (pure Rust module), and stay on the era-hardware checklist for real
   BIOS proof (follow-on stub in the overview).

## 2. Normalization into `MemoryRegion[]` (pure module, host unit-tested)

Input: raw detected entries. Output: the ABI's sorted, non-overlapping array (≤
`MEMORY_MAP_MAX_ENTRIES`, overflow is a hard error, never truncation) written to its final
home `0x0003_0000`:

1. Map E820 types: 1 → `Usable`; everything else (ACPI both flavors, NVS, unknown) → `Reserved`.
2. Clamp/split the first megabyte exactly to the reservation table (`00-boot-abi.md` §5):
   IVT/BDA `Reserved`, stage windows `BootloaderReclaimable`, `0x80000–0xFFFFF` forced `Reserved`
   even when the BIOS claims otherwise (E820 lies on era firmware).
3. Above 1 MiB: intersect out the kernel/initrd/page-table zone once those exist (phases 8–11
   extend the input; the normalizer keeps carve-out a parameter, not a special case).
4. Merge adjacent regions of identical kind; assert sorted-by-base, zero overlaps, zero
   zero-length entries (a violation is a stage2 `[ERR] memnorm` + exit 3, never silent).
5. `DisplayInfo`: constant `VgaText` per the ABI table (no probing mode-sets; the text window at
   `0xB8000` is a contract, not a discovery).

## 3. Verification (gate rewritten — the original "usable_bytes == -m" is false by construction:
E820 always subtracts the 640 KiB–1 MiB hole, `0x9FC00`, and top-of-RAM pages)

New `boot-test` rows print the full normalized map, one region per line
(`[MEM] base=… len=… kind=…`), then a summary
(`[MEM] entries=N raw_sum=X norm_sum=Y usable=Z OK`):

1. `-m 32M` and `-m 512M` (both `-cpu 486`, boot32 media). Harness-side checks on the printed
   map: (a) `raw_sum == norm_sum` — normalization redistributed, never lost or invented;
   (b) sorted + non-overlapping recomputed from the print; (c) `usable == -m − 1 MiB − bios_gap`
   where `bios_gap` is also printed, harness asserts `bios_gap ≤ 4 MiB`; (d) every reservation-
   table carve-out present with its exact kind.
2. Host unit tests: normalizer against synthetic inputs — out-of-order entries, adjacent
   same-kind merges, an E820 claiming `0x80000` usable, an E801-only machine, an AH=88-only
   machine, a >64-entry overflow refusing hard, the `0x1C20` block-size quirk.
3. Prior rows + audits green.

**Pass criteria:** all rows green; unit suite green; commit.
