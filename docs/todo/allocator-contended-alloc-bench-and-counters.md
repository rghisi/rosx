---
priority: normal
date: 2026-11-01
trigger: none
---

# Allocator: measure contention before optimizing (plans/userspace-allocator.md M4, item 0)

Bins are per-image statics, so contention exists only among `Spawn` siblings of one image (bench, stress, threads). Every alloc/free pays two atomic RMWs (`usrlib/src/allocator/mod.rs`) even in a single-task program, and locked instructions are expensive under QEMU.

Add an `alloc_contended` suite (N `Spawn` siblings alloc/free the same class, report aggregate pairs/s next to the single-task `alloc_cached` number) plus a few allocator counters (slabs held, chunks per bin, lock-contention yields) exposed through kstatsd so `mem` can show them. This is the prerequisite data and acceptance oracle for the two allocator todos that follow.
