---
priority: normal
date: 2026-11-21
trigger: any stage2 change that adds a heap allocation (a new file lookup, a deeper path, a bigger read window, or x86_64-flavor work)
---

# stage2's bump heap never frees; the 64-bit flavor runs at 85%

`arch/x86-bootloader/stage2/src/heap.rs` is a pure bump allocator with a no-op `dealloc`, so every transient `Vec` the `rxfat` engine allocates during a lookup leaks for the rest of the boot. Measured peaks (2026-11-21, temporary instrumentation, debug-port stream): **39,296 / 65,536 bytes** on the Flat32 disk-boot flow, **55,552 / 65,536** on the stage-64 flavor — which is why giving stage2 a deeper kernel path (`/BOOT/KERNEL.ELF`) OOMs the 64-bit chain: each extra path component costs another directory-read `Vec` (~10 KiB), and stage2-64 held only ~10 KiB of headroom.

The window cannot simply grow: `stage2` lives in `0x7E00..0x2FFFF` (hard linker `ASSERT`), the bounce buffer ends at `HEAP_ADDRESS = 0x20000`, and the 64 KiB heap already ends exactly at `0x30000`.

Cure candidates, in rough order of appetite: a LIFO tail-free bump allocator (free only the latest allocation, recovering most lookup churn); a reusable directory buffer handed to `rxfat` (engine API change); chunked directory scanning in `rxfat` itself (engine-internal, no allocation per directory). Prove any cure with `cargo xtask boot-test` green **both flavors** plus the measured peaks re-printed on the debug stream.
