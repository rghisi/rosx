---
priority: low
date: 2026-11-01
trigger: observed fragmentation pain or mid-size workloads becoming normal
---

# Allocator: slab trim, return fully-free slabs to the kernel

v1 never frees slabs back; exit-time group reclaim is the net, so a program that spikes once keeps its high-water mark for life.

**Snag.** Slabs are fetched with `Syscall::alloc(SLAB_BYTES, SMALL_MAX)`, 4096-aligned, so a chunk address does not identify its slab, and the design bans per-chunk metadata. Cheapest fixes: raise the slab-fetch alignment to 65536 so the slab base falls out of a chunk pointer by masking the low 16 bits, or keep a small slab-base roster inside `Binner`. Trim then detects a fully-free slab (checkpoint: just before fetching a new one), unthreads its chunks from the class bin, and `Dealloc`s it.

**Dependency.** Best composed with the dlmalloc-style arena (`plans/userspace-allocator.md` section 5): a coalescer absorbs trimmed slabs and re-hands out merged memory. Together they are one future memory-economy project.
