---
status: decided
date: 2026-11-27
revisit-when: the raised counts can be driven back down
---

## Question
Why was the `unsafe` baseline raised?

## Chosen
Tier 1 slab heap (K2): realloc routing and trim (retire_page, release_arena, arena_of, empty_arenas) in kernel/src/memory/slab_heap.rs, rationale in docs/rationale/kernel-slab-heap.md

- kernel: 361 -> 389

## Alternatives
Fixing the cause so the baseline stays put.

## Consequences
The `unsafe` ratchet now allows the counts above.
