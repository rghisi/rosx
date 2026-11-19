---
status: decided
date: 2026-11-19
revisit-when: rationale comments multiply or the baseline stops shrinking
---

## Question
Should the "no comments, ever" rule stay absolute?

## Chosen
No comments except a short `///` or `//!` block (at most 6 lines) on an item containing `unsafe`/`asm!`/`global_asm!`, or an `unsafe fn`/`unsafe impl`, that links a rationale document under `docs/rationale/`. Every other comment is banned. Existing comments are frozen in a baseline that only shrinks. Assembly files keep their mandatory comments.

## Alternatives
- Keep the absolute ban: leaves genuinely low-level code (context switch, spinlock ordering) unexplained.
- Baseline-only freeze with no new comments: the earlier decision, refined by this one.

## Consequences
`check-style` enforces the rule; every rationale file must be linked from at least one doc comment.
