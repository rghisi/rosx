# Memory ownership

## Invariants

- User blocks are stamped `BlockOwner::Group`; `LoadElf` and boot loads start a fresh group and `Spawn` siblings join the caller's group.
- The ELF image and its argv live in the group's single `ProgramMemory` block, loaded eagerly at the load call; the kernel retains no ELF bytes after a successful load and a task's first slice parses or loads nothing.
- A group's memory is reclaimed when its last live member terminates.
- Task stacks carry `BlockOwner::Stack(group)`: accounted to the group but freed by the owning task's `Drop`; `deallocate_by_owner` matches `Group` only, never `Stack`.
- The initrd is a `BlockUse::Held` block: threaded onto no free list, counted in `total_bytes` but never in `free_bytes`, and never allocatable, until `MemoryManager::release_held()` (`rom::release` is its only caller) coalesces it into the heap after bootstrap.
- `GenerationalArena` grows on demand in 32-slot pages and never shrinks; the cap passed to `new` is the owning subsystem's quota and `add` surfaces `OutOfMemory` rather than aborting.
- The kernel heap is two tiers: the slab heap (Tier 1) serves every `GlobalAlloc` request that `small_class` covers (size ≤ 512 B, alignment at most the class's largest power-of-two divisor) and draws its arenas from Tier 0 as `BlockOwner::Kernel` blocks; `FreeListAllocator` (Tier 0) stays the byte-granular supplier of everything else. Routing is the pure function `small_class(&Layout)`, identical on `alloc` and `dealloc`.
- Which policy serves a `GlobalAlloc` request lives in the `KernelMemoryAllocator` injected into `MemoryManager::new` at construction; the kernel runs `KernelSlabAllocator`, which owns the `SlabHeap`. `MemoryManager` holds only the interrupt guard, the Tier 0 borrow, and owner-block accounting, and passes `&mut FreeListAllocator` to the policy per call; a different kernel policy implements the trait and changes nothing else.
- Owner-tagged blocks never reach Tier 1: `Group` and `Stack` blocks are allocated through `alloc_with_owner` and freed only through `dealloc_owned` or `deallocate_by_owner`; Tier 1 owns no user memory and `group_bytes` and `deallocate_by_owner` read Tier 0 alone.
- Tier 1 detects a bad free with O(1) checks only (header magic, header class equals the layout's class, slot offset a multiple of the class size, slot index inside capacity, bitmap bit set) and then leaks the slot; a wild pointer that happens to carry the magic into a same-class slab is not caught. A fully-free slab page is retired to its arena with its magic cleared, and a fully-free arena returns to Tier 0 unless it is the single spare. Tier 0's live-list checks are unchanged.

## Design

**Memory ownership = loaded-image groups** — user blocks are stamped `BlockOwner::Group`: `LoadElf` (and boot loads) starts a fresh group, `Spawn` siblings join the caller's group (they share that image's statics), and the kernel reclaims a group's memory when its last live member terminates. Task stacks carry `BlockOwner::Stack(group)` — accounted to the group but freed by the owning task's `Drop`, never by the bulk reclaim: reclaim runs while the last member's `Task` still lives, so stacks must stay out of its sweep (`deallocate_by_owner` matches `Group` only, never `Stack`).

Loading is eager and fallible at the syscall: `new_elf_task_with_arguments` inspects the image,
allocates one group-stamped `ProgramMemory` block laid out `[image | program record | argv table |
argument strings]`, and writes every byte in place; `elf_task_wrapper` only reads the record and
jumps, so no ELF bytes are kept anywhere after the load and the first slice validates nothing. A
malformed ELF fails `LoadElf` — kernel diagnostic, `u64::MAX` to the caller, group swept — and can
never panic the kernel; only the boot-time init load panics, where there is no one left to report
to.

**Two-tier kernel heap** — small objects must not pay a general allocator's cost (a 32 B header on
x86_64, a first-fit walk to allocate, a live-list plus two free-list walks to free). Which policy
serves a request is its own object: `MemoryManager` is built around an injected
`KernelMemoryAllocator`, today `KernelSlabAllocator` (`kernel/src/memory/kernel_allocator.rs`),
which owns the slab and routes by `small_class`; the manager keeps the interrupt guard, the Tier 0
borrow, and owner accounting. Tier 1 (`kernel/src/memory/slab_heap.rs`)
is a size-class slab heap: the class table and `SMALL_MAX = 512`
are fixed by the K0 histogram in `plans/memory-tiers-results.md`; arenas are 16 page-aligned pages
drawn from Tier 0 as one `Kernel` block; each slab page carries its header at the page end so a slot
address masks straight to it, slots start at the page base (so a class's largest power-of-two
divisor is free alignment), and a page sits on its class partial list exactly while it has free
slots but is not full. Allocate and free are O(1); Tier 0 failure to supply an arena returns null,
never a panic. K2 returns fully-free pages to their arena and fully-free arenas to Tier 0, keeping
one spare arena so a boundary workload does not thrash; `free_bytes` in `kstatsd` therefore counts
the spare arena as used. A same-class `realloc` keeps the pointer; any other size change routes an
inline alloc-copy-free across the tiers. Layout, alignment rule, trim/hysteresis and the weaker
bad-free story: `docs/rationale/kernel-slab-heap.md`.
