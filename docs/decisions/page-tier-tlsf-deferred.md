---
status: open
date: 2026-11-22
revisit-when: K3 of plans/memory-tiers.md shows Tier 0 allocate/free cost or fragmentation is still material for stacks, program memory or user Alloc blocks
---

# Page tier under the kernel allocator (TLSF, out-of-band descriptors) deferred

## Question

Should the kernel get a dedicated page allocator beneath the byte-granular allocator, with page-level
ownership, instead of keeping `FreeListAllocator` as the lowest tier?

## Chosen

Not now. `FreeListAllocator` stays as Tier 0 and a size-class slab heap (Tier 1) is added above it
(`plans/memory-tiers.md`). Once small objects leave Tier 0 its live-block count falls to tens, which
removes the O(live) free pathology without changing ownership, the `Alloc` ABI semantics or what `mem`
reports. Work continues on that default; K3 of the plan re-measures and decides.

## Alternatives

- **TLSF over page-sized blocks with out-of-band tags.** Boundary tags (size, free flag, owner, list
  links) live in a descriptor array, one entry per frame: the first descriptor of a run holds the run,
  the last holds a back-pointer for coalescing. O(1) allocate and free, page-aligned runs with no
  in-band header, free pages never written, and alignment above a page costs only whole-page gaps.
  Ownership moves to the descriptors, so group reclaim becomes a descriptor sweep or per-group chain.
  Costs: about 0.2 to 0.4% of RAM for descriptors, per-region descriptor slices for non-contiguous
  memory, page-granular `Alloc` and `mem` numbers (a semantic change that needs approval at the
  `system` boundary), and rewriting `BlockOwner` handling and `docs/architecture/memory.md`.
- **Buddy allocator for pages.** Fast with natural alignment, but up to 50% waste for sizes that are
  not powers of two, which suits runs of pages (stacks, images) worse than TLSF.
- **TLSF replacing the free list as one tier.** Bounded latency and a one-word header, but no help for
  tiny objects (four-word minimum block) and over-alignment needs over-allocate-and-split.

## Consequences

- Tier 0 keeps its 32 B header (16 B on x86_32) per block and O(live + free) free; with few live blocks
  this is acceptable, and K3 measures it.
- Ownership stays per block (`BlockOwner`), `group_bytes` stays exact, `mem` is unchanged.
- If K3 reopens the question, the TLSF-with-descriptors design above is the starting point, and the
  `Alloc` semantic change must be approved before it lands.
