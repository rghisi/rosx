# BootInfo consumer at the x86 kernel edge

## Why this is hard
`kernel::platform::bootinfo::usable_blocks` reads a `system::boot_info::BootInfo` — and the
memory map behind its `memory_map_ptr` — straight out of low physical memory handed over
by stage2. The wire bytes are not typed memory: `BootArch`, `BootMethod`, `MemoryKind`
and `DisplayKind` carry invalid discriminants until validated, so a Rust struct read
would be UB, and a wild map pointer is a boot-time fault with no MMU to contain it.

## Invariants relied on
- Every field is copied width-exactly little-endian from raw bytes at
  `core::mem::offset_of!` positions of the frozen treaty types; no wire value is ever
  reinterpreted as an enum discriminant. `MemoryKind::from_wire` is the only enum decode.
- Validation happens once here, before any dereference of the map: magic, version,
  arch, `memory_map_entry_size` (must equal `size_of::<MemoryRegion>()`),
  `memory_map_len` (at most `MEMORY_MAP_MAX_ENTRIES`, so the map slice is bounded at
  64 × 24 bytes), a non-zero, addressable `memory_map_ptr`, and a
  `DisplayKind::VgaText` display at `0xB8000` — the only console the kernel actually
  drives, asserted from the treaty rather than re-derived.
- The two `from_raw_parts` slices (header, map) are the caller's safety contract: the
  producer (stage2, or a test buffer) must keep those bytes readable for the call.
- Only `MemoryKind::Usable` regions become blocks; regions are clipped at `kernel_end`
  and dropped (reported) when unaddressable, zero-length, or beyond the
  `kernel::memory::MAX_MEMORY_BLOCKS` cap — nothing fails silently.

## What breaks if changed
Loosening a gate above re-introduces either UB (bad discriminant, wild pointer) or a
kernel that allocates over its own image. The treaty itself is frozen
(`docs/architecture/boot-plan.md`, `system/src/boot_info.rs`): if a future producer
bumps `BOOT_INFO_VERSION`, this consumer must reject it loudly, and the dual-magic
entry in the arch crate is where the failure becomes visible on both console streams.
