# Memory ownership

## Invariants

- User blocks are stamped `BlockOwner::Group`; `LoadElf` and boot loads start a fresh group and `Spawn` siblings join the caller's group.
- The ELF image and its argv live in the group's single `ProgramMemory` block, loaded eagerly at the load call; the kernel retains no ELF bytes after a successful load and a task's first slice parses or loads nothing.
- A group's memory is reclaimed when its last live member terminates.
- Task stacks carry `BlockOwner::Stack(group)`: accounted to the group but freed by the owning task's `Drop`; `deallocate_by_owner` matches `Group` only, never `Stack`.
- The initrd is a `BlockUse::Held` block: threaded onto no free list, counted in `total_bytes` but never in `free_bytes`, and never allocatable, until `MemoryManager::release_held()` (`rom::release` is its only caller) coalesces it into the heap after bootstrap.
- `GenerationalArena` grows on demand in 32-slot pages and never shrinks; the cap passed to `new` is the owning subsystem's quota and `add` surfaces `OutOfMemory` rather than aborting.

## Design

**Memory ownership = loaded-image groups** — user blocks are stamped `BlockOwner::Group`: `LoadElf` (and boot loads) starts a fresh group, `Spawn` siblings join the caller's group (they share that image's statics), and the kernel reclaims a group's memory when its last live member terminates. Task stacks carry `BlockOwner::Stack(group)` — accounted to the group but freed by the owning task's `Drop`, never by the bulk reclaim: reclaim runs while the last member's `Task` still lives, so stacks must stay out of its sweep (`deallocate_by_owner` matches `Group` only, never `Stack`).

Loading is eager and fallible at the syscall: `new_elf_task_with_arguments` inspects the image,
allocates one group-stamped `ProgramMemory` block laid out `[image | program record | argv table |
argument strings]`, and writes every byte in place; `elf_task_wrapper` only reads the record and
jumps, so no ELF bytes are kept anywhere after the load and the first slice validates nothing. A
malformed ELF fails `LoadElf` — kernel diagnostic, `u64::MAX` to the caller, group swept — and can
never panic the kernel; only the boot-time init load panics, where there is no one left to report
to.
