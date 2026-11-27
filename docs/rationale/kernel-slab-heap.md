# Tier 1 kernel slab heap

## Layout

One arena is `ARENA_PAGES` (16) contiguous 4 KiB pages, page-aligned, fetched
from Tier 0 as one `BlockOwner::Kernel` block. The arena's page 0 holds the
arena header (`magic`, `next`, `pages_free`, `used_pages` bitmap); pages
1..16 are slabs. A slab page serves exactly one size class: slots start at
the page base and the `SlabHeader` (`magic`, `class`, `free_count`,
`free_head`, `prev`, `next`, `[u32; 12]` bitmap) sits at the page end, so
`slot_count = (4096 - size_of::<SlabHeader>()) / class` and the header
budget is ≤ 96 B (the class-16 bitmap alone is 32 B).

## Why header at the end

Slots starting at the page base means a slot's address masks straight to
its page (`ptr & !(4096 - 1)`), and the header is at a fixed offset inside
the page, so `deallocate` finds it with no lookup. A power-of-two class then
gets natural alignment at every slot (the page base is 4096-aligned), so
alignment up to the class's largest power-of-two divisor is free; a request
needing more takes a wider class or falls back to Tier 0 (`small_class`).

## Bad-free story

Tier 1 checks, in O(1) after an arena-membership scan: header magic, header
class equal to the layout's class, slot offset a multiple of the class size,
slot index inside capacity, and the slot's bitmap bit set. A wild pointer
that happens to carry the magic into a same-class slab is not caught — the
old Tier 0 live list did not defend against aliasing either, but Tier 1
narrows the window by not consulting any per-allocation table. Failures log
`BAD-FREE slab <reason>` (first ten) and leak the slot.

## Invariants relied on

- Routing is the pure function `small_class(&Layout)`; `alloc` and `dealloc`
  of the same layout always land on the same tier. Owner-tagged blocks
  (`Group`, `Stack`) are allocated by `alloc_with_owner` and freed by
  `dealloc_owned`/`deallocate_by_owner` and never see Tier 1. The
  `LoadElf`/`IpcConnect`/`IpcBind` handlers free their `kernel_box` argument
  cells through `dealloc_owned` for the same reason: user space allocated
  them with the `Alloc` syscall, so a `Box::from_raw` drop would free a
  Tier 0 block through the Tier 1 route.
- A slab page is on its class partial list exactly while `free_count > 0`;
  the transition `0 → 1` links it, `→ 0` unlinks it. Empty slabs stay on
  the list (trim is K2); arenas are never returned.
- Single core, kernel runs allocator calls with IF=0 (`MemoryManager`
  guards), so no locking and no atomics in Tier 1.

## What breaks if changed

A `dealloc` routed by a different predicate than `alloc` frees a block the
other tier never gave out. Writing a slot's free-list link into an allocated
slot (a missed bitmap check) corrupts the class free chain. Growing arenas
while the `MemoryManager` cell is re-borrowed would alias the allocator;
Tier 1 therefore takes `&mut FreeListAllocator` as a parameter.

## Evidence

The unit tests in `kernel/src/memory/slab_heap.rs` (model:
`kernel/src/scheduler/mlfq_strategy.rs`), including a randomized
alloc/free model over a plain buffer, and the boot logs of
`cargo xtask test` on both arches with `BAD-FREE` silent.

## Status

verified.
