---
priority: low
date: 2026-11-01
trigger: none
---

# Allocator: realloc tail extension

The same-class no-op `realloc` (`realloc_same_class` in `usrlib/src/allocator/bins.rs`) has landed. Remaining: in-place extension across a class boundary when the chunk is the most-recently-carved tail of its slab and the next region is untouched. It needs a per-slab carve frontier that `adopt` currently discards by threading every chunk into the free list. Optional: the win is only tail growth, and the returned pointer must be valid for the new layout (routing symmetry, `plans/userspace-allocator.md` section 3.2).

Trap: `GlobalAlloc::realloc(self, ...)` path-calls resolve to the type's own override, not the trait default; a fallback must inline the default semantics, never delegate through the trait.
