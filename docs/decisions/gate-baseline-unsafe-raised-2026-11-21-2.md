---
status: decided
date: 2026-11-21
revisit-when: the raised counts can be driven back down
---

## Question
Why was the `unsafe` baseline raised?

## Chosen
elf-load-at-syscall M0: two characterization tests exercise the unsafe allocator API (GlobalAlloc::alloc / release_group_allocations) to prove the image is not group-owned and measure today's image alignment

- kernel: 300 -> 302

## Alternatives
Fixing the cause so the baseline stays put.

## Consequences
The `unsafe` ratchet now allows the counts above.
