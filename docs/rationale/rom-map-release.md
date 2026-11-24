# RomMap/RomRelease: one flat mapping of the ROM image

## Why this is hard
The ROM (INITRD.BIN) sits in RAM that the kernel could hand out again. If the allocator
reuses those pages while `init` is still reading program images out of the ROM, ELF loads
succeed against garbage. The old path paid for safety with copies: every `rom:` step walked
`RomRead` (4 KiB per syscall) into fresh heap, twice over for the hand-off image. The fix
removes the copies, so the only thing standing between `init` and freed memory is a
mapping the kernel hands out and can take back.

## Invariants relied on
- The image's pages are a single `BlockUse::Held` block from `bootinfo::usable_blocks`.
  The allocator never threads a `Held` block onto the free list, so nothing can allocate
  under the ROM while any map could exist.
- `kernel::rom` owns the lifecycle `Held → Mapped(owner group) → Released`
  (`Lifecycle` in `kernel/src/rom.rs`). `RomMap` records the caller's group once and is
  idempotent for that group; `RomRelease` flips the held block to `Free` (coalesced by
  `insert_free_block`) exactly once, for the owner only, and moves to `Released`.
  Every refusal carries a distinct `ErrorCode` (`RomReleased`, `RomNotOwner`,
  `RomAlreadyReleased`) and mutates nothing. After `Released`, `image()` serves an empty
  slice so even the kernel cannot read the freed bytes.
- Validation lives once, kernel-side. User space carries the opaque `RomRef` and re-checks
  nothing; `usrlib::rom::RomView` is the whole user-side contract: `image(&self)` borrows
  the mapped bytes for the view's life and `release(self)` consumes the view, so a
  use-after-release in `init` is a borrow-check error, not a runtime hazard.
- `init` releases at the end of its own control flow (hand-off or plan exhaustion), after
  its last `LoadElf` returned; `LoadElf` is eager and retains no ROM bytes.

## The flat-address-space assumption
`RomMap` returns a raw address (`RomRef.start` is the kernel slice pointer). That is
readable from every current user task because both x86 arches run one flat address space:
a single page table with an identity window (x86_64 also aliases it higher-half), no
user/supervisor page split, and no protection features that would fault a user read. If
user space ever moves out of that window (ring 3 with its own tables, SMEP/SMAP, WP set),
`RomMap` must map pages into the reader instead of aliasing them.
`docs/decisions/rom-map-assumes-flat-address-space.md` (status: open) tracks that revisit.

## What breaks if the invariant fails
A `Held` block flipped to `Free` at the wrong time is the deliberate S2 boot probe: the
allocator takes the ROM's first bytes as block bookkeeping and the boot dies with
`initrd is not a ROM image: BadMagic`. A release before the last `LoadElf` is caught the
same way — the ROM bytes are garbage by then, and the load fails loudly rather than
executing corrupted pages.
