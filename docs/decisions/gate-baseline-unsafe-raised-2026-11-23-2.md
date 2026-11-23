---
status: decided
date: 2026-11-23
revisit-when: the raised counts can be driven back down
---

## Question
Why was the `unsafe` baseline raised?

## Chosen
RomMap out-pointer write and release_held insert_free_block in kernel; usrlib RomView::image from_raw_parts with its fabricated-RomRef test

- kernel: 302 -> 305
- usrlib: 62 -> 65

## Alternatives
Fixing the cause so the baseline stays put.

## Consequences
The `unsafe` ratchet now allows the counts above.
