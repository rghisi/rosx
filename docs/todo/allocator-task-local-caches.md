---
priority: low
date: 2026-11-01
trigger: allocator-contended-alloc-bench-and-counters shows atomics or contention cost is material
---

# Allocator: task-local caches behind a TaskId syscall

A per-task, per-class LIFO pocket in front of the shared bins: alloc/free pop and push task-private memory unsynchronized; the global spinlock survives only on batch refill/overflow. Win: contention removal for multi-task images and atomics removal for single-task ones.

**Prerequisite.** `TaskId = 38` returning `current_task().pack()`: a tiny ABI addition (`system`, both arch dispatchers, usrlib), independently useful for diagnostics and a future `ps`.

**Sketch.** A plain `static` in the flat address space is shared, not task-private, so pockets live in a small global slot table indexed by low bits of the `TaskId`, with the owner's full `TaskId` (generation included) stored in the slot header.

**Design crux, slot liveness.** The kernel cannot clean a user-space slot when its owner dies. Ownership is holding the list pointer: a foreign claimant atomically exchanges the slot head to null and walks away with the whole list, so a stolen pocket can only lose free chunks, never handed-out ones, and an owner whose CAS fails refills from the shared bin. Memory ownership itself stays group-scoped (`BlockOwner::Group`).
