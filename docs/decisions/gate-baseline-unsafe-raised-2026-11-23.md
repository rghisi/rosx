---
status: decided
date: 2026-11-23
revisit-when: the raised counts can be driven back down
---

## Question
Why was the `unsafe` baseline raised?

## Chosen
held-block allocator unit tests call unsafe allocate/deallocate

- kernel: 299 -> 302

## Alternatives
Fixing the cause so the baseline stays put.

## Consequences
The `unsafe` ratchet now allows the counts above.
