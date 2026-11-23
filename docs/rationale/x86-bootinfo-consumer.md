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

## The initrd slice
`initrd` adds one more `from_raw_parts`: the initrd bytes themselves, over the range
the BootInfo names. Its safety contract is stronger than the header's: the producer
(stage2) keeps those bytes readable, and the check that the whole physical range lies
inside one `KernelAndModules` region of the published map is what proves the kernel
allocator can never hand that memory out — the map never offers the region as `Usable`.
The x86_64 address arrives through the higher-half direct-map alias, so the containment
test subtracts `HIGHER_HALF_OFFSET` before comparing against the physical map.
What breaks if loosened: a wild or aliased initrd address becomes a live slice the
allocator may also write into — silent corruption of the ROM the kernel boots from.
Width is enforced by `usize::MAX` of the compiled target: exact on x86_32, vacuous on
the host, so the host tests prove the 32-bit range logic by value while the x86_32 and
x86_64 QEMU boots are the proof that the address spaces really carry the slice.
(The x86_64 higher-half acceptance is host-tested on metadata only; the readable
alias itself is proven by the QEMU boot.)

## The held initrd block

`usable_blocks` and `initrd` share one `initrd_range` check: the ABI row (the absent zero
pair yields `Ok(None)`), the physical address through the arch's alias, the
target-width bound, and the requirement that the whole physical range lie inside one
`KernelAndModules` region. `initrd_range` dereferences no pointer itself — the caller
builds the `header` and `map` slices and passes them in — so it is a plain function, and
only the `from_raw_parts` at the consumer edges are `unsafe`.

After the usable blocks, `usable_blocks` appends one `MemoryBlock` tagged
`BlockUse::Held` covering the page-rounded initrd range (start = physical initrd start,
size = `align_up_page(start + size)`, i.e. the `place_payload` end stage2 reserved). This
block is inside `regions` (so `region_contains` still accepts a pointer there and
`deallocate` validation is unchanged) but `FreeListAllocator::new` threads only
`free_blocks()`, so the ROM is never on the free list and can never be handed out. A full
block table counts the held block as `dropped`, never lost. `MemoryMapFacts.held` is
printed as `[BOOTINFO] … held=…` because `[BOOTINFO] blocks=` counts accepted usable
regions only and cannot show the held block. Flipping that tag to `Free` hands the ROM to
the allocator and the boot panics `initrd is not a ROM image` — the tag is the protection.
The held range returns to the free list only when `init` releases it (see
`docs/rationale/rom-map-release.md`).
