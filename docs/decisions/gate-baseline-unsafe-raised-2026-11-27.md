---
status: decided
date: 2026-11-27
revisit-when: the raised counts can be driven back down
---

## Question
Why was the `unsafe` baseline raised?

## Chosen
Tier 1 slab heap (K1): raw slab and arena writes in
kernel/src/memory/slab_heap.rs and the routed allocator paths in
kernel/src/memory/memory_manager.rs, plus the `kernel_box` free fix in
kernel/src/syscall.rs; rationale in docs/rationale/kernel-slab-heap.md

- kernel: 306 -> 361

## Alternatives
Fixing the cause so the baseline stays put.

## Consequences
The `unsafe` ratchet now allows the counts above.
