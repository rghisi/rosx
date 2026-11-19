---
status: decided
date: 2026-11-19
revisit-when: none
---

## Question
Where do the long per-subsystem design paragraphs of `AGENTS.md` live?

## Chosen
In `docs/architecture/<subsystem>.md`, one file per subsystem. `AGENTS.md` keeps one-line invariants and an index.

## Alternatives
Keep everything in `AGENTS.md` (56 KB, goes stale while claiming to be always-accurate).

## Consequences
`AGENTS.md` shrinks to roughly 12 KB; every removed paragraph must exist in exactly one architecture file.
