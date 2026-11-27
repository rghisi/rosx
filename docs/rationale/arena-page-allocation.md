# Paged arena allocation

## Why this is hard
`kernel::collections::arena::GenerationalArena` grows in fixed 32-slot pages during syscalls that run
with IF=0. A page built with `Box`/`Vec` cannot report allocation failure: the global allocator's
error handler aborts the kernel when a `Box::new`-style allocation returns null. To surface
`OutOfMemory` (the documented quota path) instead of aborting, `allocate_page` calls `core::alloc::alloc`
directly, which returns null on exhaustion without consulting the handler.

## Invariants relied on
- The layout used by `alloc` (`Layout::array::<Slot<T>>(count)`) is the layout the
  `Box<[Slot<T>]>` built via `Box::from_raw` deallocates with on drop; `count` never changes after
  the page enters the directory.
- Every slot is fully written (generation 0, `next_free` set, `item` `None`) before the block is
  pushed into `pages`, so no uninitialized `Slot<T>` is ever reachable, and dropping a page drops
  only initialized slots.
- `count` is `1..=32` and never zero (`grow` refuses at the cap first), and `count <= max_slots <= 65535`
  keeps the index arithmetic inside the 16-bit handle space.

## What breaks if changed
Reading `pages` before the slot loop completes exposes uninitialized memory (UB). A mismatched
alloc/dealloc layout corrupts the kernel free list. Treating allocation failure as unreachable
re-introduces the abort the plan forbids on the `add` path.

## Evidence
The unit tests in `kernel/src/collections/arena.rs` (`cargo test -p kernel -- --test-threads=1`),
including the cap path where `add` returns `OutOfMemory` across a page boundary.

## Status
verified.
