---
status: decided
date: 2026-11-27
revisit-when: the raised counts can be driven back down
---

## Question
Why was the `unsafe` baseline raised?

## Chosen
KernelMemoryAllocator policy extraction: the five routed unsafe calls (tier0 allocate/deallocate,
slab allocate/deallocate, realloc copy) moved inline in `MemoryManager`'s `GlobalAlloc` paths now
live in the slab policy `kernel/src/memory/kernel_allocator.rs`, and the pass-through test policy
adds two more; the manager's routed paths keep only their `unsafe fn` declarations. Rationale in
docs/rationale/kernel-slab-heap.md.

- kernel: 389 -> 391

## Alternatives
Fixing the cause so the baseline stays put.

## Consequences
The `unsafe` ratchet now allows the counts above.
