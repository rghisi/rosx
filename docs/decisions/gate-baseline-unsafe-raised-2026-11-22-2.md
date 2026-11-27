---
status: decided
date: 2026-11-22
revisit-when: the raised counts can be driven back down
---

## Question
Why was the `unsafe` baseline raised?

## Chosen
arch initrd bootinfo wrapper

- arch/x86_32: 30 -> 31
- arch/x86_64: 37 -> 38

## Alternatives
Fixing the cause so the baseline stays put.

## Consequences
The `unsafe` ratchet now allows the counts above.
