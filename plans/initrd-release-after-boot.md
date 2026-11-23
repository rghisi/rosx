# Initrd release — `init` maps the ROM zero-copy and hands its RAM back

> **Status:** Proposed. Written 2026-11-23 on branch `claude/initrd-release-bootstrap-gu4ust` at
> `origin/main` = `1f10d56` (#199). Nothing is implemented.
> **Goal:** once bootstrapping has finished (mass storage mounted, hand-off done), the RAM that
> holds the initrd — placed by stage2 right after the kernel and carved out of the allocator's
> reach — returns to the kernel allocator. Along the way `init` stops copying the ROM: the kernel
> hands it a reference (start + length) and `init` reads, and `LoadElf`s, straight from it.
> **Method:** every code fact is cited to a path re-verified against this tree; facts the plan
> cannot prove at authoring time are marked **unverified** and routed to the step that must
> prove them.
> Repo law applies throughout: `kernel/` stays portable and `no_std`; the kernel is the single
> authority on the ROM's lifecycle; no comments outside the frozen baseline (the one allowed
> shape: a ≤ 6-line `///` linking its rationale on an item containing `unsafe`); commits bare
> imperative; every step lands green through `cargo xtask check`; the series ends with
> `cargo xtask verify` on a quiet host.
> **User decisions already taken (2026-11-22/23):** a `RomRelease` syscall managed by `init`; the
> kernel provides only start and size with zero copies; the allocator learns about the initrd at
> initialization through a tag on `MemoryBlock` (usable vs releasable), with no allocator API
> beyond one release method on `MemoryManager`.

---

## 0. Goal & Non-Goals

### Goal
- The initrd range reaches the allocator at bootstrap as a `MemoryBlock` tagged `Held`: inside
  `regions` (so `deallocate` validation is unchanged) but never on the free list.
- Two new syscalls replace `RomRead`: **`RomMap`** reports the image's `RomRef { start, len }` and
  records the calling group as the ROM's owner; **`RomRelease`**, owner-only and once only, retires
  the image and gives the held block to the free list.
- `init` holds one borrowed view of the ROM for the whole boot plan, passes ROM sub-slices
  directly to `LoadElf` (the kernel's one eager copy into group memory stays — it is what lets
  the kernel retain no ELF bytes), and releases the ROM when it is done: after a successful
  hand-off, or when the plan is exhausted.
- After release `kernel::rom::image()` is empty and `RomMap` fails with a named error; a second
  `RomRelease`, or one from a non-owner, fails with a named error and changes nothing.

### Non-Goals
- **No `boot_plan` grammar change.** Release rides `init`'s own control flow, not a new plan
  step (D5). `system::boot_plan` is untouched.
- **No kstats wire change.** Held bytes are reported by a kernel log line, not by
  `MemorySnapshot` (`system::kstats` is a wire contract).
- **No bootloader, `BootInfo` or ROM-format change.** stage2 still places `INITRD.BIN` at
  `kernel_phys_end` and `initrd_addr/initrd_size` are consumed unchanged.
- **No ring-3 claim.** x86_64 runs user ELFs in ring 0 today (`arch/x86_64/src/syscall.S:58`:
  "we don't support Ring 3 yet"), so a raw pointer is a valid reference; a decision record owns
  the day that stops being true (D6).
- **No `rom:` → `initrd:` rename** and no compression; both stay as in
  `plans/rom-as-initrd.md`.
- **No new third-party dependency, no widening of the 486 feature floor.**

---

## 1. Verified baseline facts

All re-verified against `1f10d56`.

- **The initrd is outside the allocator by construction.** stage2 streams `INITRD.BIN` to
  `kernel_end` with `place_payload` (`arch/x86-bootloader/common/src/memnorm.rs:231`:
  `address = kernel_end`, `end = align_up_page(kernel_end + size)`) and carves
  `[kernel_phys_start .. initrd.end]` as `KernelAndModules`
  (`stage2/src/main.rs:116`). The image starts exactly at `kernel_end`: **no gap** before it.
  The kernel validates the range sits inside one `KernelAndModules` region
  (`kernel/src/platform/bootinfo.rs::initrd`, `InitrdNotReserved`) and builds a `'static` slice
  over it; on x86_64 the wire address is the higher-half alias (`wire - HIGHER_HALF_OFFSET` is
  the physical one, `bootinfo.rs:181`).
- **Boot order.** `kernel_main` calls `kernel::kernel::bootstrap(&memory_blocks, …)`
  (`kernel/src/kernel.rs:1129` → `MEMORY_MANAGER.bootstrap`) *before* it reads the initrd
  (`arch/x86_64/src/main.rs:62-70`, same shape in `x86_32`). The map is therefore consumed
  twice: `usable_blocks` for the allocator, then `initrd` for the slice.
- **The allocator.** `FreeListAllocator::new(&MemoryBlocks)` threads every block into an
  intrusive, address-sorted free list and keeps `*memory_blocks` as `regions`
  (`kernel/src/memory/free_list_allocator.rs:102-123`). `regions` is only read by
  `region_contains` (validation of frees). **`insert_free_block` (line 303) coalesces with both
  its successor and its predecessor** and rejects overlap with an existing free entry — so a
  released held block merges with the heap block that starts right after the carve.
- **`MemoryBlock { start, size }`** has 11 struct-literal sites; `MemoryBlocks` is consumed in
  `arch/x86_{32,64}/src/main.rs`, `kernel/src/platform/bootinfo.rs`, `kernel/src/kernel.rs`,
  `kernel/src/memory/{mod,free_list_allocator,memory_manager}.rs` and nowhere else
  (`bitmap_chunk_allocator.rs` keeps its own `Region` type). `MAX_MEMORY_BLOCKS = 32`.
- **The ROM today is a copy-heavy path.** `usrlib::rom::read_image()` copies the whole image
  into a heap `Vec` at 4 KiB per `RomRead` syscall (`usrlib/src/rom.rs:7-19`); `init` holds it
  for the whole plan (`apps/init/src/main.rs:19`); `program_image()` then does
  `Vec::from(bytes)` per `rom:` step (`main.rs:44-52`) before `LoadElf`.
- **User space and the kernel share one flat address space.** `LoadElf` takes a boxed
  `&[u8]` raw pointer from user space (`kernel/src/syscall.rs:118-135`) and `RomRead` writes
  through a raw user pointer (`syscall.rs:220-223`). A `RomRef` pointer is thus directly
  readable by `init`.
- **`LoadElf` is eager and retains nothing** (`docs/architecture/memory.md`): after it returns,
  no ROM bytes are referenced by the loaded group, so the ROM is releasable once `init`'s last
  load has returned.
- **`kernel::rom`** is `Once<&'static [u8]>` + `program`/`validate`/`read`/`read_at`
  (`kernel/src/rom.rs`); `read`/`read_at` are the only `RomRead` backing.
- **Syscall numbers** live in `system/src/syscall_numbers.rs` with a round-trip test; the
  enum has gaps (13, 14, 16–19, 21), i.e. retired numbers are left unreused. **Unverified:**
  that this is a deliberate rule — S0 confirms before 27 is retired.
- **Unsafe ledger.** `check-style` baselines `unsafe` per crate in `xtask/style-baseline.txt`;
  raising a count needs a decision record in the same commit
  (`check-style --update-baseline --allow-increase "<reason>"`).

### Unverified, routed to S0
- **U1 — address space of block starts on x86_64.** `usable_blocks` receives `safe_start =
  &kernel_phys_end` (`arch/x86_64/src/main.rs:121`); whether block starts are physical
  (identity-mapped low 4 GiB) or higher-half decides which alias the `Held` block must carry so
  it matches every other block. The `RomRef` the user sees is the higher-half alias; the held
  block is whichever alias the allocator already uses.
- **U2 — ROM size per arch** and the free-heap figure before/after, as the evidence baseline.
- **U3 — block-cap headroom.** One more block (the initrd) against `MAX_MEMORY_BLOCKS = 32`;
  the `dropped` counter in `MemoryMapFacts` must stay 0 on the boot media.
- **U4 — whether `init` is single-task** for its whole life (the kernel enforces the owner
  *group*, not the task).

---

## 2. Decisions

- **D1 — `MemoryBlock` gains a required tag `use: BlockUse { Free, Held }`.** No `Default`
  (a forgotten tag must be a compile error, not a silent hand-out of the ROM).
  `MemoryBlock::free(start, size)` covers the 11 literals; `MemoryBlocks::free_blocks()` yields
  only `Free` blocks and is what `FreeListAllocator::new` threads onto the free list. All blocks
  (both tags) go into `regions`.
- **D2 — One entry point: `MemoryManager::release_held()`.** It flips every `Held` block to
  `Free` and `insert_free_block`s it (coalescing with the heap). Idempotent: with nothing held it
  returns 0. Returns the released byte count for the log line. `rom::release` is its only
  caller. (Rejected: `add_region()` — mutates `regions` at runtime and can fail on capacity;
  rejected: header-in-block treatment of the ROM as an allocation — the image starts at
  `kernel_end` with no gap, so an `AllocHeader` would overwrite its first bytes and fixing that
  changes the boot-media layout.)
- **D3 — The initrd range reaches the allocator through `usable_blocks`.** The validation in
  `bootinfo::initrd` (range inside one `KernelAndModules` region, width checks) is factored into
  one shared function used by both `usable_blocks` (which appends the `Held` block, start =
  initrd start, size = `align_up_page(size)` — i.e. `place_payload`'s `end`) and `initrd`
  (which keeps returning the slice). One validation, two consumers; no ordering change in the
  arch mains.
- **D4 — `kernel::rom` owns a three-state lifecycle: `Held` → `Mapped(owner group)` →
  `Released`.** `RomMap` from `Held` records the caller's group; from `Mapped` it succeeds only
  for that same group (idempotent); from `Released` it fails `RomReleased`. `RomRelease`
  succeeds only in `Mapped` and only for the owner, calls `MEMORY_MANAGER.release_held()`,
  clears the image slice, prints `[ROM] released addr=… bytes=…`, and moves to `Released`;
  everything else fails with a distinct `ErrorCode` and mutates nothing. The kernel's own
  boot-time `rom::program("init")` happens before any map. Validation lives once, kernel-side;
  user space carries the opaque `RomRef` and re-checks nothing.
- **D5 — `init` releases at the end of its own control flow; no plan step.** Every `rom:` token
  is resolved (and its image eagerly copied into group memory by `LoadElf`) before `init`
  returns, and `exec` ends the plan, so releasing on a successful hand-off — or at plan
  exhaustion — is always after the last ROM read. The `rom:shell` fallback is itself an `exec`
  loaded before release, so it survives any plan failure. (Revisit if a plan ever needs an early
  release: record in `docs/decisions/`.)
- **D6 — Raw-pointer `RomRef` is valid because the user/kernel space is flat.** It is a `system`
  layout struct (numbers and layouts only — the treaty rule). `usrlib` wraps it as
  `RomView<'a>` whose `image()`/`find()` borrow `&self` and whose `release(self)` consumes it,
  so the borrow checker makes use-after-release unrepresentable in `init`; `RomView` is built by
  `RomView::map()`, the one new `unsafe` (`from_raw_parts`) in `usrlib`, behind a ≤ 6-line `///`
  linking `docs/rationale/rom-map-release.md`. If user space ever moves to ring 3 the syscall
  must map pages instead; `docs/decisions/rom-map-assumes-flat-address-space.md` (status open,
  `revisit-when: user space runs outside ring 0`) records that.
- **D7 — `RomRead` is deleted, not kept.** `init` is its only caller; production API with no
  caller is deleted (AGENTS.md). Its number 27 is retired (left unused); `RomMap` and
  `RomRelease` take new numbers appended after `Kill = 42`.
- **D8 — `program_image` returns a borrow, not a `Vec`, for ROM tokens.** An
  `enum Image<'a> { Rom(&'a [u8]), Owned(Vec<u8>) }` with `as_slice()`; VFS reads stay owned.
  Net copies per ROM launch: three or more → one (the kernel's eager load).

---

## 3. Steps

Each step is green through `cargo xtask check`, committed bare-imperative with the repo's
attribution trailer, pushed to the working branch, and ends with the `retro` skill's step-end
mode reported with the step's commit. A lesson changing rules or docs is a proposal that lands
only after approval, in its own commit. The PR is a user-requested act.

### S0 — Baseline and proofs (no production code)
- Quiet-host gate by a script file (`pgrep -x 'qemu-system-x86'`, load vs `nproc`).
- Build both arches (`cargo xtask build`, `cargo xtask build x86_32`; logs to files with exit
  codes) and boot each once to the shell prompt; record `INITRD.BIN` size, the `mem` free-bytes
  figure and `[BOOTINFO] … blocks=… dropped=…` (U2, U3).
- Settle U1 by reading `usable_blocks` + the x86_64 `boot_info_memory` consumer and, if unclear,
  a one-off host test printing the block starts; settle U4 by reading `apps/init`; confirm the
  retired-number convention in `system/src/syscall_numbers.rs` history (`git log -p`).
- Output: a "S0 findings" section appended below ("Execution records"). A finding that
  contradicts a decision in §2 stops the series and amends this plan first.

### S1 — Allocator: the tag and `release_held` (`kernel/`, host-tested; no behaviour change)
- `kernel/src/memory/mod.rs`: `BlockUse`, the required `use` field, `MemoryBlock::free`,
  `MemoryBlocks::free_blocks()`; migrate the 11 literals.
- `free_list_allocator.rs`: `new` threads only `free_blocks()`, keeps all blocks in `regions`;
  add `release_held()`. `memory_manager.rs`: `MemoryManager::release_held()` under the manager's
  existing guard discipline (`IrqCell`/`KernelCell` as its neighbours use).
- Unit tests (model: `mlfq_strategy.rs`): a held block is never returned by `allocate` however
  large the request; `region_contains` still accepts a pointer inside it and `deallocate` of such
  a pointer logs `not-in-alloc-list` and changes nothing; `release_held` makes the bytes
  allocatable; it coalesces with the adjacent heap block (`free_block_count` unchanged, bytes
  grown); it is idempotent (second call returns 0); stats `free_bytes` rise by exactly the
  held size.
- No held blocks exist yet, so boot behaviour is unchanged.

### S2 — BootInfo: the initrd range arrives as a `Held` block (`kernel/`, both arches boot)
- `bootinfo.rs`: factor the range validation (D3); `usable_blocks` appends the `Held` block
  after the usable ones (cap-checked: a full table counts as `dropped` and the arch main's
  existing warning fires); `initrd` reuses the factored check. Extend
  `docs/rationale/x86-bootinfo-consumer.md` with the held-block contract.
- Host tests over the fabricated window: initrd present → exactly one extra `Held` block whose
  range equals the page-rounded initrd; absent initrd → no block; straddling/not-reserved/width
  cases keep their existing errors (the 32-bit overflow cases by value, per
  `plans/rom-as-initrd.md` R4).
- Evidence: both arches boot to the shell; `[BOOTINFO] blocks=` is exactly one higher than S0's;
  `mem` free bytes equal S0's (the ROM is held, not free). One deliberate check: temporarily
  flip the tag to `Free` and confirm the boot now corrupts or the first test fails — proof the
  tag is what protects the ROM; revert before commit.

### S3 — The flip: `RomMap` / `RomRelease`, zero-copy `init` (one commit, both arches)
Atomic because deleting `RomRead` and moving `init` off it cannot be split without a broken
boot.
- `system`: `SyscallNum::{RomMap, RomRelease}` (appended; `RomRead` removed, round-trip test
  updated), `system::rom::RomRef` (`#[repr(C)] { start, len }`, layout assertions as
  `system::boot_info` does), `ErrorCode` variants for `RomReleased`, `RomNotOwner`,
  `RomAlreadyReleased` (names finalised against `system/src/error.rs`'s wire-numbering rule).
  **ABI/wire change — approved in principle by the user; the exact numbers and codes are listed
  in the commit and PR for review.**
- `kernel/src/rom.rs`: replace `read`/`read_at` and the bare `Once` with the D4 state machine;
  `kernel/src/syscall.rs`: `RomMap` writes the `RomRef` through the user's out-pointer,
  `RomRelease` calls `rom::release(current_group_owner())`; `RomRead` arm deleted.
- `usrlib`: `Syscall::rom_map`/`rom_release`; `usrlib::rom::RomView` (D6); `read_image` and
  `Syscall::rom_read` deleted. `docs/rationale/rom-map-release.md`.
- `apps/init`: `RomView::map()` once; `Image<'a>` (D8); release on successful hand-off and at
  plan exhaustion (D5); the `[INIT]` diagnostics keep their text.
- Unit tests (kernel): state machine — map by A, map by B refused, map by A again ok, release by
  B refused, release by A ok and returns bytes, map after release `RomReleased`, second release
  refused; `rom::program` unaffected before map. Unit tests (`usrlib`): `find` over a fabricated
  `RomRef`; the handle-kind guard test (`usrlib/tests/handle_kind_guards.rs`) still passes.
- Gate: `check-style --update-baseline --allow-increase "RomView::map from_raw_parts; kernel
  release_held insert"` writes the decision record in this commit (unsafe counts for `usrlib`
  and, if raised, `kernel`).

### S4 — Integration proof, both arches
- `tests-integration`: boot to the shell prompt and assert the `[ROM] released addr=… bytes=…`
  line appears exactly once, after the last `[INIT] …` boot-plan line and before the shell
  prompt on both arches (`ROSX_ARCH=x86_32` too); assert `[IRQGUARD]` stays silent and no
  `BAD-FREE` line appears (the release path must not trip the allocator's validation).
- A functional proof of reuse: after boot, run a command whose allocation exceeds the largest
  pre-release free block only if the ROM coalesced (sized from S0's figures; if S0 shows that
  cannot be arranged robustly, replace with the `[ROM] released` + `mem` free-bytes delta
  asserted ≥ `INITRD.BIN` size minus page rounding).
- Failure-path proof: a boot plan with a deliberately missing `/shell` still reaches the
  `rom:shell` hand-off and then releases (log order asserted).

### S5 — Docs and records
- `docs/architecture/kstatsd-and-rom.md`: the ROM lifecycle (held → mapped → released), `RomRead`
  → `RomMap`/`RomRelease`, `init` is still the ROM's only user-space reader.
- `docs/architecture/memory.md`: the held-block tag and `release_held`; invariant line "the
  initrd is a held block, never allocatable until `init` releases it".
- `AGENTS.md` invariants index: the `kstatsd`/ROM line gains "and `init` releases its RAM after
  bootstrap".
- Decision records: `rom-map-assumes-flat-address-space.md` (open, D6) and
  `rom-release-by-init-not-plan-step.md` (decided, D5, `revisit-when: a plan needs an early
  release`).
- Close or amend any `docs/todo/` item this supersedes; none found at authoring time.
- S0's before/after numbers go into the execution note.

### V — Merge evidence
`cargo xtask verify` on a quiet host, one log per step, exit code recorded to a file (never from
piped or truncated output — AGENTS.md toolchain notes). Boot-test rows must show no new SKIPs.

---

## 4. Risks and contingencies

- **R1 — Wrong alias for the held block on x86_64 (U1).** A held block carrying the higher-half
  alias while the rest of the blocks are physical (or the reverse) would make `release_held`
  thread a free block at an address the allocator's `regions` check rejects, or in the wrong
  place. S0 settles the alias; S1's `region_contains` test and S2's boot evidence prove it.
- **R2 — A consumer ignoring the tag hands out the ROM.** Mitigated structurally: required
  field, no `Default`, `free_blocks()` as the only iteration path, S2's deliberate-flip check.
- **R3 — Use-after-release in `init`.** `RomView`'s borrow/consume shape rules it out inside
  `init`; the kernel enforces owner and once-only for everyone else. A sibling task of `init`'s
  group could still hold a `&[u8]` (U4); if `init` is ever multi-task, the release must wait for
  siblings — recorded in the S3 rationale file.
- **R4 — Release before a `LoadElf` finished.** `LoadElf` is synchronous and eager, and release
  runs on the same task after the last load returns; there is no overlap window.
- **R5 — Capacity.** One extra block against 32 (U3). A full table already degrades by
  `dropped` + a boot warning; a dropped *held* block would leave the ROM range unprotected only
  in the sense that it was never given to the allocator at all — which is the safe direction.
  S2's test pins that a dropped held block is counted, not silently lost.
- **R6 — Fragmentation.** The released range abuts the first heap block and `insert_free_block`
  coalesces, so no island is created (S1 test); if S0 shows the heap does *not* start at the
  carve's end on some arch, the release is still correct, it merely forms its own free block.
- **R7 — ABI churn.** Removing `RomRead` breaks any out-of-tree user; in-tree it has one caller
  (`init`), changed in the same commit.
- **R8 — Flakiness.** Integration evidence obeys the quiet-host rule
  (`plans/integration-flakiness-assessment-2026-11-20.md`); a contention failure is not a
  verdict.

## 5. What closes / what stays open

**Closes:** the initrd's permanent RAM cost (one ROM-sized region reclaimed per boot) and
`init`'s ROM-sized heap copy plus per-launch `Vec::from` copies.

**Stays open:**
- Ring-3 user space needs `RomMap` to map pages, not hand out a pointer (decision record,
  D6).
- Plan-driven early release, if ever needed (decision record, D5).
- Held-bytes visibility in `mem` would need a `system::kstats` wire change — not taken here.
- Hardware evidence for how much RAM this actually frees on 486-class machines
  (`docs/architecture/real-hardware-x86_32.md`); this plan claims QEMU numbers only.

## Friction log

S0 | `ROSX_KERNEL_ELF=target/rosx/release/rosx` (relative) → harness `canonicalize` failed, test
aborted | ENV-TRAP | 1 round (rerun with `$PWD/...` absolute)
S1 | `cargo xtask check` first run exit 1: `///` on the non-unsafe `BlockUse` (mod.rs comment
baseline 0) and kernel `unsafe` 299→302 from new cfg(test) `allocate`/`deallocate` calls | MISSING-FACT |
1 round (removed the comment; `check-style --update-baseline --allow-increase` wrote the record)
S3 | `cargo xtask check` first run exit 1: new `Lifecycle` owner field written as `AtomicU64`,
flagged by check-isa as a 32-bit hazard despite the AGENTS.md 486-floor line | TASK | 1 round
(fixed to `AtomicUsize`; the planned baseline raise was already routed by the plan's Gate line)

## Execution records

### S0 — Baseline and proofs (no production code)

Quiet-host gate (`pgrep -x 'qemu-system-x86'`, load vs `nproc`) held before every boot; the host
was idle each time. Both arches built (`cargo xtask build`, `cargo xtask build x86_32`) and booted
to the `rose>` prompt from the prebuilt release kernels. A throwaway `tests-integration` boot test
captured the lines and the `mem` snapshot, then was deleted; no production code changed.

ROM image size on the boot volume (`target/<arch>-user/rom.img`, what stage2 places at
`kernel_end` and the kernel sees as the initrd):

| Arch | ROM bytes | page-rounded (held block) |
|---|---|---|
| x86_64 | 573,449 | 577,536 (141 pages) = 564 KiB |
| x86_32 | 550,689 | 552,960 (135 pages) = 540 KiB |

Boot lines and `mem` snapshot at `rose>` (the before figures for S2 and S4):

| Arch | `[BOOTINFO]` | `[ROM]` | `mem` free | `mem` total | largest free | free blocks |
|---|---|---|---|---|---|---|
| x86_64 | regions=11 usable=0x7e52000 blocks=1 clipped=1 dropped=0 | addr=0xffff800000146000 size=573449 files=9 | 127355 KiB | 129052 KiB | 126195 KiB | 77 |
| x86_32 | regions=10 usable=0x7e6a000 blocks=1 clipped=1 dropped=0 | addr=0x13a000 size=550689 files=9 | 127693 KiB | 129148 KiB | 126503 KiB | 82 |

**Findings against the §1 unverified items — none contradicts a §2 decision:**

- **U1 — settled: block starts are physical; the `RomRef` carries the kernel slice pointer.**
  The `[BOOTINFO]` blocks come from the memory-map `base` fields, which are physical, and
  `safe_start` (`kernel_phys_end`) is the physical clip floor. The `[ROM]` address differs by arch:
  x86_64 reports the higher-half alias `0xffff800000146000`, x86_32 reports the physical
  `0x13a000`. So the **held block carries the physical start** (`wire − HIGHER_HALF_OFFSET` on
  x86_64, equal on x86_32) — the alias the allocator already uses — and the **`RomRef` the user
  reads is `rom::image().as_ptr()`**, i.e. the same slice pointer the kernel boots from (valid
  because user space is flat ring 0). This is what D6/R1 assumed; it holds.
- **U2 — ROM size and free-heap figures are the table above**, per arch. Releasing reclaims
  564 KiB (x86_64) / 540 KiB (x86_32).
- **U3 — block-cap headroom confirmed: `dropped=0` on both arches.** Only one usable block reaches
  the allocator today (`blocks=1`); appending one held block keeps the table at two of the
  `MAX_MEMORY_BLOCKS = 32`. No new drop.
- **U4 — `init` is single-task for its whole life.** `apps/init/src/main.rs` runs `main()` to
  completion and never calls `Spawn`, so recording the calling group as the ROM owner is
  sufficient: no sibling of init's group can outlive the release. R3's multi-task caveat stays
  recorded but does not fire today.
- **Retired-number rule confirmed by an existing test.** `system/src/syscall_numbers.rs` carries
  `try_read_char_number_stays_retired` (asserts 9 stays unused) alongside the gaps at
  13/14/16–19/21. Retiring `RomRead = 27` (left unreused) matches the convention.

S2 target checks derived here: `[BOOTINFO]` must show **`held=1`** on both arches, `dropped` must
stay **0**, and `mem` free bytes must stay **near S0's** (the ROM joins `total_bytes` but never
`free_bytes`; only the kernel-image growth of later steps nudges the figures by a few KiB).

### S1 — Allocator tag and `free_blocks` (host-tested, no behaviour change)

`MemoryBlock` gained a required `use_: BlockUse { Free, Held }` (no `Default`), a
`MemoryBlock::free` constructor, and `MemoryBlocks::free_blocks()`; all 10 struct literals migrated
to the constructor. `FreeListAllocator::new` now threads only `free_blocks()` onto the free list
while keeping every block in `regions`, so `region_contains` still accepts a pointer inside a held
range. Four unit tests prove the tag: `free_blocks` yields only `Free`; a held block is never
allocated however large the request and never counted free; `deallocate` of a held pointer is
rejected `not-in-alloc-list` and leaves the list intact; a held block counts in `total_bytes` but
not in `free_bytes`. No held block is produced yet, so boot is unchanged.

**Deviation from §3 (rule-driven, not a §2 change):** `release_held` is deferred to S3. `kernel` is
a lib crate, so a `release_held` with only test callers is a banned test-only production surface
(AGENTS.md), and a `pub(crate)` one is dead in the binary build; its only real caller,
`rom::release`, lands in S3. Its coalescing / idempotence / `free_bytes`-rise tests therefore move to
S3 with it. The kernel `unsafe` baseline rose 299→302 for the three new test `allocate`/`deallocate`
calls, recorded in `docs/decisions/gate-baseline-unsafe-raised-2026-11-23.md`.

### S2 — The initrd arrives as a `Held` block (both arches boot)

`usable_blocks` now runs one shared `initrd_range` check (D3) and appends a `Held` block for the
page-rounded initrd range (start = physical initrd start, end = `align_up_page(start + size)`), after
the usable blocks. A full table counts the held block as `dropped`, not lost. `initrd` reuses the same
`initrd_range` (its ABI row, width, and `KernelAndModules` containment are unchanged).
`MemoryMapFacts` gained a `held` field and the arch `[BOOTINFO]` line prints `held=`, so the held
block is observable at boot (`[BOOTINFO] blocks=` alone cannot show it — it counts accepted usable
only). Four host tests cover it: reserved initrd → exactly one `Held` block page-rounded to `0x2000`;
absent row → no held block; cap full → held counted as `dropped`; plus the `held` fact.

Boot evidence (prebuilt release kernels, `rose>` reached, `mem` snapshot), and the flip check:

| Arch | `[BOOTINFO]` (S2) | `[ROM]` | `mem` free | `mem` total | flipped-to-`Free` |
|---|---|---|---|---|---|
| x86_64 | blocks=1 **held=1** dropped=0 | addr=0xffff800000147000 size=573449 | 127351 KiB | 129612 KiB | panic `initrd is not a ROM image: BadMagic` |
| x86_32 | blocks=1 **held=1** dropped=0 | addr=0x13c000 size=550689 | 127685 KiB | 129680 KiB | — |

`total_bytes` rose by the page-rounded initrd (x86_64 +560 KiB, x86_32 +532 KiB — the held block minus
the few KiB the kernel image grew between S0 and S2), while `free_bytes` stayed at S0's figure
(within that same kernel-growth noise), proving the ROM is held, not free. The deliberate flip to
`Free` on x86_64 made the allocator hand out the ROM during boot and the kernel panicked at
`main.rs:72` before reaching the shell — direct proof the `Held` tag is what protects the ROM. Reverted
before commit.

### S3 — `RomMap`/`RomRelease` and the zero-copy `init` (both arches boot and release)

One atomic step, both arches. **ABI** (D7, values as approved): syscall `27` (`RomRead`) retired
with a `rom_read_number_stays_retired` test; `RomMap = 43`, `RomRelease = 44` appended after
`Kill = 42`; error domain `0x09` — `RomReleased = 0x0901`, `RomNotOwner = 0x0902`,
`RomAlreadyReleased = 0x0903`; `system::rom::RomRef { start: u64, len: u64 }` `#[repr(C)]` with
its layout pinned by `rom_ref_layout_is_pinned` (size 16, align 8, offsets 0/8).

`kernel::rom` replaced `Once` + `read`/`read_at` with the D4 lifecycle as a small `Lifecycle`
struct (state + owner atomics) behind the module. `map` records the caller group from `Held` and
is idempotent for that group; `claim_release` succeeds only `Mapped` for the owner; `image()`
serves an empty slice once `Released`. `release` calls `MemoryManager::release_held()` (which
flips every `Held` region to `Free` and `insert_free_block`s it, coalescing and idempotent,
returning the byte count) and prints `[ROM] released addr=… bytes=…`. `RomMap` in `syscall.rs`
writes the `RomRef` through the user out-pointer and records `current_group_owner()`; both arms
refuse a non-group caller with `RomNotOwner`.

`usrlib::rom::RomView` borrows the mapped bytes (`image(&self)`) and consumes itself on
`release(self)`, so use-after-release is a borrow error in `init`; `read_image`/`rom_read` are
deleted. `apps/init` maps once and threads an `Image<'a> { Rom(&'a [u8]), Owned(Vec<u8>) }` so a
`rom:` step hands `load_with_args` a borrowed slice instead of a `Vec::from` copy, releasing at
the end of `main` after the plan runs (D5).

**Two rule-driven implementation choices, no §2 change:** (a) the lifecycle is a `Lifecycle`
struct instantiated fresh per unit test, not bare module atomics — bare globals would be poisoned
by the first test that flips them, making the state-machine tests order-dependent; (b) the owner
is an `AtomicUsize`, not `AtomicU64` — the check-isa gate rejects `AtomicU64` as a 486 hazard
(the x86_32 baseline is atomics of at most 32 bits). `caller.value()` is `usize`, so `usize` is
both the correct width and the ISA-safe one.

Tests. Kernel state machine: `map` by A ok, by A again ok, by B `RomNotOwner`; `claim_release` by
B `RomNotOwner`, by A ok, second `RomAlreadyReleased`; map after release `RomReleased`; `rom::program`
unaffected before any map. `release_held`: coalesces with the adjacent heap (free-block count
unchanged, bytes grown), makes a detached held block allocatable, idempotent (second call 0), and
nothing-held returns 0; the manager-level test proves `free_bytes` rises by exactly the held size
once and `total_bytes` is unchanged. usrlib: `find` reads program bytes through a fabricated
`RomRef`. `handle_kind_guards` still passes.

Gate: `check-style` raised `unsafe` — kernel 302→305 (`RomMap` out-pointer write, `release_held`
`insert_free_block`, two `cfg(test)` held probes), usrlib 62→65 (`RomView::image` `from_raw_parts`,
the `cfg(test)` `from_reference` and its call site) — recorded in
`docs/decisions/gate-baseline-unsafe-raised-2026-11-23-2.md`. `cargo xtask check` all green.

Boot evidence (prebuilt release kernels, throwaway `tests-integration` boot test deleted after):

| Arch | `[ROM] released` line | release position | `mem` free after release | vs S0 |
|---|---|---|---|---|
| x86_64 | addr=0xffff800000147000 bytes=577536 | after last `[INIT]`, before `rose>` | 127898 KiB | +543 KiB (ROM 564 KiB) |
| x86_32 | addr=0x13c000 bytes=552960 | after last `[INIT]`, before `rose>` | 128224 KiB | +531 KiB (ROM 540 KiB) |

The release fires exactly once, its `addr=` equals the boot `[ROM] addr=` in the same run, and
`bytes=` is the page-rounded held size (141 / 135 pages), i.e. slightly more than the raw ROM
image. `mem` free rose by within a few KiB of the held block (the shortfall is the shell and
daemons allocating after the release point). No `BAD-FREE`, `[IRQGUARD]` silent, `rose>` reached.
The boot-log address (`147000`) matches S2's, confirming the release reports the region the boot
handed over, not a later shift.
