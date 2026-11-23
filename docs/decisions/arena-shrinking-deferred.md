---
status: open
date: 2026-11-23
revisit-when: boot-time peak and steady-state usage diverge noticeably, or a subsystem cap above 1024 lands
---

## Question
Should `GenerationalArena` free pages that have become empty, or is grow-only enough?

## Chosen
Grow-only: a materialized page stays allocated until the whole arena is dropped; freed slots queue on the FIFO free list and are reused before a new page is mapped.

## Alternatives
Releasing empty pages on drain; shrinking on a memory-pressure callback; a hierarchical-bitmap free finder that would make reuse order the only growth driver.

## Consequences
Steady-state kernel heap tracks the high-water mark of each arena, bounded by its cap; no page churn or dangling-page handling inside IF=0 syscalls; a subsystem whose usage is bursty pays its peak forever.
