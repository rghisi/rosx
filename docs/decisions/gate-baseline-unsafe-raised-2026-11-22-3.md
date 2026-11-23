---
status: decided
date: 2026-11-22
revisit-when: the raised counts can be driven back down
---

## Question
Why was the `unsafe` baseline raised?

## Chosen
GenerationalArena pages come from the raw allocator so growth inside IF=0 syscalls surfaces OutOfMemory instead of aborting (docs/rationale/arena-page-allocation.md)

- kernel: 299 -> 300

## Alternatives
Fixing the cause so the baseline stays put.

## Consequences
The `unsafe` ratchet now allows the counts above.
