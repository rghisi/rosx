---
status: decided
date: 2026-11-19
revisit-when: the raised counts can be driven back down
---

## Question
Why was the `unsafe` baseline raised?

## Chosen
Extracting unsafe-bearing helpers (seed_task_frame, Once::store_value/stored_value) so their rationale lives in linked doc comments adds declaration tokens; the unsafe operations themselves are unchanged.

- arch/x86_32: 28 -> 30
- kernel: 289 -> 293

## Alternatives
Fixing the cause so the baseline stays put.

## Consequences
The `unsafe` ratchet now allows the counts above.
