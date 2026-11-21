---
status: decided
date: 2026-11-21
revisit-when: the raised counts can be driven back down
---

## Question
Why was the `unsafe` baseline raised?

## Chosen
dual-magic boot entry: the BootInfo consumer reads the physical handoff window (kernel) and kernel_main calls it (arch/x86_32); every access sits behind the validated kernel::platform::bootinfo API

- arch/x86_32: 30 -> 31
- kernel: 295 -> 300

## Alternatives
Fixing the cause so the baseline stays put.

## Consequences
The `unsafe` ratchet now allows the counts above.
