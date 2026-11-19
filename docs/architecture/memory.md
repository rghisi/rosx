# Memory ownership

## Invariants

- User blocks are stamped `BlockOwner::Group`; `LoadElf` and boot loads start a fresh group and `Spawn` siblings join the caller's group.
- A group's memory is reclaimed when its last live member terminates.
- Task stacks carry `BlockOwner::Stack(group)`: accounted to the group but freed by the owning task's `Drop`; `deallocate_by_owner` matches `Group` only, never `Stack`.

## Design

**Memory ownership = loaded-image groups** — user blocks are stamped `BlockOwner::Group`: `LoadElf` (and boot loads) starts a fresh group, `Spawn` siblings join the caller's group (they share that image's statics), and the kernel reclaims a group's memory when its last live member terminates. Task stacks carry `BlockOwner::Stack(group)` — accounted to the group but freed by the owning task's `Drop`, never by the bulk reclaim: reclaim runs while the last member's `Task` still lives, so stacks must stay out of its sweep (`deallocate_by_owner` matches `Group` only, never `Stack`).
