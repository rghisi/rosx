---
status: open
date: 2026-11-23
revisit-when: a target or boot medium imposes a kernel image size limit, a measured reap exceeds 1 ms on x86_32 hardware or emulation, or another change needs to touch the future, IPC or timer indexes anyway
---

## Question
Should the kernel drop every `BTreeMap` (ten fields, `plans/kernel-btreemap-removal.md`) now, to cut the B-tree code from both images?

## Chosen
Defer the full removal. The plan stays on file and is not scheduled; `by_group` and the other nine maps stay as they are.

Measured on `main@f85af9f`, release x86_64: 69 unique B-tree functions take 68.8 KB of 210 KB `.text` (33%), which confirms the assessment. x86_32 is not measured. Nothing in the repo today is short of space (idle boot use is 1.7 MiB), so the saving has no consumer yet.

## Alternatives
- Execute the plan now: saves up to about 69 KB of `.text` (less the replacement code), at the price of scans on teardown paths that run with IF=0, and a second rewrite of `future/mod.rs` one day after #202.
- Remove one map at a time: little gain on its own. The measurement shows maps with the same key layout share one copy of the code (a `GroupId` map and a `u64` map resolve to the same address on x86_64), so the code goes away only when the last instantiation goes.
- Replace only the timer with a `BinaryHeap`: worth doing on behaviour, not size (nothing due costs one comparison and no allocation, FIFO among equal deadlines by sequence number). It can land on its own and does not need this decision to change.

## Consequences
The kernel image keeps about 69 KB of generic B-tree code. No scan regressions are added to task exit and kill. The plan's Step 0 still has to be measured on x86_32 before any step starts, and its figures must be re-based on the then-current head.
