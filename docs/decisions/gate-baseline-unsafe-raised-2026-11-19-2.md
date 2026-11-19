---
status: decided
date: 2026-11-19
revisit-when: the raised counts can be driven back down
---

## Question
Why was the `unsafe` baseline raised?

## Chosen
Merged main #176 (Ctrl+C / Kill syscall) adds two unsafe tokens to kernel; the code is not part of this branch's work.

- kernel: 293 -> 295

## Alternatives
Fixing the cause so the baseline stays put.

## Consequences
The `unsafe` ratchet now allows the counts above.
