# Phase 7 — FAT Storage: `rxfat` over the Thunk (Decision D3 lands here)

Stage2 reads real files off the boot volume by implementing the workspace engine treaty on top of
the BIOS — not by re-writing a FAT parser. The engine is `rxfat` (FAT16/FAT32, total boot-sector
parser, differential-tested against mtools in both directions, zero third-party deps), reached
through `fs_core::BlockDevice` exactly the way `vfsd` reaches it in production.

## 1. Implementation

- **`BiosBlockDevice`** (`stage2/src/blk.rs`): implements the `fs_core::BlockDevice` treaty for
  the **partition at LBA 2048** (base offset baked in at construction — the engine only ever
  speaks volume-relative units, same discipline as `blockd`'s slice remapping). Every request
  flows through the thunk's `INT 13h`:
  - LBA media: `AH=42h` DAP transfers chunked at ≤ 64 sectors per call (era controller sanity,
    and the chunk stays inside the 127-sector CHS-compatible ceiling).
  - CHS media: sector-at-a-time `AH=02h` with the phase-3 retry policy.
  - Transfer buffer: the bounce window `0x0004_0000` (the map array at `0x30000` is written once
    at phase 6 and never touched by disk traffic — the original plan's bounce/map collision is
    dead by construction in the reservation table).
- **Bump allocator**: `#[global_allocator]` over a fixed arena (64 KiB) carved from the bounce
  window; `rxfat`'s handle/chain structures allocate, nothing else does; exhaustion is a hard
  `[ERR] heap` exit-3, never a hang.
- **Mount & read**: `rxfat::FileSystem::new(device)` on the volume; `lookup`/`open`/`read` for
  `INITRD.BIN` into a flat staging area **above 1 MiB** (start at `0x0010_0000` for now — the
  kernel-load relocation arrives phase 8; the staging address is printed every run so the
  harness never guesses). Engine errors (`Unsupported` FAT12/foreign boot sector, `Corrupt`)
  print `[FAT-ERR] <kind>` and exit 3 — the honest-error culture is inherited, not re-implemented.
- The x86_64 boot image's volume carries the same `KERNEL.ELF`/`INITRD.BIN` layout from pack-time
  on, but nothing reads ELF until phase 8/11.

## 2. Verification

1. `fat-initrd` on `boot32.img` (`-m 32M`): stage2 reads `INITRD.BIN` (64 KiB seeded random from
   the packager), prints `[FAT] initrd addr=… size=65536 crc32=0x…`; harness recomputes the
   expected CRC32 from the recorded seed and asserts equality end to end — first full
   disk→engine→memory integrity proof. Exit 33.
2. `fat-corrupt`: image whose FAT root region is scribbled after packing → `[FAT-ERR]` present,
   no `[FAT]` success line, exit 3 (not timeout, not exit 1).
3. `fat-boot64-media`: same read proven on `boot64.img`'s volume (pack-time identical FAT16
   recipe) under `qemu-system-x86_64 -cpu qemu64` — proves device-layer arch-independence early.
4. Stage2 size gate re-checked with `rxfat` + `fs-core` linked in (≤ 160 KiB budget; if it
   breaks, the response is `-C opt-level`/`lto` tightening in the boot profile first, the
   budget's story updated second — both facts recorded in the commit).
5. Host unit tests: `BiosBlockDevice` chunking logic against a fake INT13 backend (offsets at
   the partition base, chunk boundaries at the volume end, the 64-sector ceiling).

**Pass criteria:** rows 1–4 green; the engine crates themselves untouched (`cargo test -p rxfat`
green unchanged — if a change seems needed, stop and escalate: stage2 adapts, the treaty doesn't).
