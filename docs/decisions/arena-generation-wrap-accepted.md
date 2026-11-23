---
status: decided
date: 2026-11-23
revisit-when: a handle-retirement scheme is proposed for another reason, or a slot is seen wrapping in practice
---

## Question
Slot generations are 15-bit and advance on every free, so a stale handle aliases again after 32 768 reuses of the same slot. Is that window acceptable?

## Chosen
Accept the wrap with no retirement tracking.

## Alternatives
Wide generations with a widened `Handle` (rejected: the `Handle` bit layout is out of scope); retiring slots until every holder drops them (rejected: the kernel retains no registry of who holds a handle).

## Consequences
Stale-handle rejection is exact for 32 768 reuses of one slot and probabilistic beyond that. Reaching the window needs tens of thousands of alloc/free cycles on the very slot a stale holder still pokes — far beyond any workload the current arenas see under their 1024 caps.
