# Memory tiers — a kernel small-object tier over the existing allocator, then a finer user tier

> **Date:** 2026-11-22 · **Branch:** `ccr-067ee909-tqabe6` · **Base:** `origin/main@162e69f`
> **Status:** proposal, approved in discussion; steps land one commit (or one short series) each.
> **Evidence base:** a read of `kernel/src/memory/{free_list_allocator,memory_manager,program_memory}.rs`,
> `kernel/src/{syscall,task_stack,kstatsd}.rs`, `usrlib/src/allocator/`, `docs/architecture/memory.md`,
> `plans/userspace-allocator.md` and the two `docs/todo/allocator-*` items. Every byte figure below is
> computed from the type definitions or estimated from reading the code; K0 and U0 produce the real
> numbers, and where they disagree with this document the numbers win.

## Goal

Small allocations stop paying for a general-purpose allocator. The kernel heap gets a size-class slab
tier (Tier 1) with O(1) allocate and free, no per-object header and natural alignment up to the class
size; the existing `FreeListAllocator` stays exactly as it is and becomes Tier 0, the byte-granular
supplier of large blocks, stacks, program memory, user `Alloc` blocks and Tier 1's own arenas. The user
allocator in `usrlib` is then refined on its own track (finer classes, O(1) slab lookup, slab trim).
The two tiers are separate implementations on purpose: they evolve for different reasons.

## Today

- **Kernel, one tier.** `FreeListAllocator` is a first-fit, address-ordered free list plus a linked list
  of live allocations. `AllocHeader { size, owner: BlockOwner, next }` is 32 B on x86_64 and 16 B on
  x86_32 (`BlockOwner` is a tag plus a `usize`); sizes round up to `BLOCK_ALIGN` (8 / 4), so a 1-byte
  request costs 40 B on x86_64 (`ALLOC_HDR + 8`).
- **Cost shape.** `allocate` walks the free list (`free_list_allocator.rs:125`). `deallocate` walks the
  live list to find the header (`:200`), then `insert_free_block` walks the free list twice, once to
  probe for overlap and once for the sorted insert (`:303`). Free is O(live + free blocks) with
  interrupts off, so it degrades as small allocations accumulate. `deallocate_by_owner` and
  `group_bytes` walk the live list too.
- **Alignment.** Up to `BLOCK_ALIGN` is free. Above it, `plan_carve` places the header directly before
  the aligned pointer plus one back-reference word, and returns the front sliver to the free list when
  it is at least `BLOCK_HDR`. Nothing is lost permanently, but it fragments.
- **Callers of the owner-tagged path** (`alloc_with_owner`): the user `Alloc` syscall
  (`syscall.rs:110`, `Group`), `TaskStack::new` (`task_stack.rs:39`, `Stack`) and `ProgramMemory`
  (`program_memory.rs:24`, `Group`). Everything else, the `Box`/`Vec`/`Arc` heap, arrives through
  `GlobalAlloc::alloc` and is stamped `Kernel`. `dealloc` ignores `layout.size()` and passes only the
  alignment.
- **Stacks are large.** `STACK_WORDS` is 8192 words, 64 KiB on x86_64, and 12288 words, 48 KiB on
  x86_32, with `usize` alignment. They are not small objects and do not move.
- **Consumers of the stats.** `kstatsd` reports `total/free/largest_free/free_block_count` from Tier 0
  and `group_bytes` for the process table (`kernel/src/kstatsd.rs:57,78`). The unit tests in
  `kernel/src/kernel.rs` only assert group bytes are zero or non-zero.
- **User space, already two tiers.** `usrlib/src/allocator/`: nine power-of-two classes (16 B to
  4 KiB) cut from 8 KiB slabs fetched with `Syscall::alloc(SLAB_BYTES, chunk)`, intrusive free lists,
  zero per-chunk metadata, one global spinlock that yields on contention. Worst-case internal waste is
  50%, slabs are never returned (`docs/todo/allocator-slab-trim.md`), and anything above 4 KiB or
  over-aligned is one syscall into Tier 0.
- **Environment.** Kernel code runs with IF=0 on one core; the address space is flat, so a "page" is
  only a software unit. `MemoryManager` already brackets every allocator call with interrupt
  disable/enable.

## Decisions taken

1. **Tier 0 is kept as is.** Once small objects leave it, its live-block count falls to stacks,
   program memory, user blocks and Tier 1 arenas (tens, not thousands), which removes the pathology
   without touching ownership, the `Alloc` ABI semantics or `mem`. The page-granular alternative is
   recorded as a deferral (`docs/decisions/page-tier-tlsf-deferred.md`).
2. **Tier 1 is a size-class slab heap, one 4 KiB page per slab, header at the end of the page.** A slot
   then starts at the page base, so a power-of-two class gets natural alignment, and masking the
   pointer finds the header. No third-party code, no new crate, `kernel/` still depends only on
   `system`.
3. **Routing is a pure function of the layout**, as in `usrlib`: `small_class(layout)` is `Some` when
   `size <= SMALL_MAX` and the class can honour the alignment; otherwise the request goes to Tier 0.
   `dealloc` receives the layout, so it routes with no lookup and `realloc` across a class boundary is
   allocate, copy, free.
4. **Tier 1 draws page-aligned arenas from Tier 0.** A page-aligned request to Tier 0 burns up to a
   page in front of it (header and back-reference sit before the aligned pointer, the gap becomes a
   sliver), so asking per page would strand about half the memory. Asking for a 16-page arena amortises
   the sliver to about 6%. Arena size is a tunable.
5. **No locking in Tier 1.** IF=0 on one core; the existing guard in `MemoryManager` stays and Tier 1
   runs inside it. Tier 1 methods take `&mut FreeListAllocator` as a parameter instead of reaching a
   global, so they are unit-testable over a plain buffer and never re-borrow the manager's cell.
6. **Ownership is unchanged.** Arenas are `BlockOwner::Kernel` blocks in Tier 0; `Group` and `Stack`
   blocks never touch Tier 1.
7. **Bad-free detection gets weaker and says so.** Tier 1 checks, in O(1): header magic, header class
   equal to the layout's class, slot offset a multiple of the class size, and the slot's bitmap bit.
   It does not defend against a wild pointer into memory that happens to carry the magic; the old live
   list did. Recorded in the K1 decision record.
8. **`SMALL_MAX`, the class table and the arena size are data-driven.** Defaults below stand until K0
   reports the real size distribution.
9. **User tier is its own implementation** in `usrlib/src/allocator/`; nothing is shared with the
   kernel tier.

## Tier 1 defaults (revisited by K0)

- `PAGE = 4096`, `ARENA_PAGES = 16` (page 0 holds the arena header and a page-state bitmap, 15 slab
  pages), `SMALL_MAX = 512`.
- Classes: 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 384, 448, 512.
  Worst-case rounding waste is under 25%, typically about 12%. Illustration on x86_64: 1 B costs 16 B
  instead of 40 B, 24 B costs 24 B instead of 56 B, 100 B costs 112 B instead of 136 B.
- Slab header at the end of the page, at most 64 B: magic, class, free count, free-list head,
  partial-list links, slot bitmap. About 1.6% of the page; the last slot ends where the header begins.
- Guaranteed slot alignment is the largest power of two dividing the class size; a request needing
  more goes to Tier 0.
- Zero-size layouts keep today's behaviour (Tier 0 refuses them).

## Measurement protocol (K0, K3, U0, U4)

The harness is disposable, as in `plans/adaptive-arenas.md`: uncommitted patches kept in the scratchpad
with result files, so `check-style` (which surveys `git ls-files`) cannot trip and nothing test-only
lands in production. Results are committed as `plans/memory-tiers-results.md` only.

**Kernel traffic (K0, needs QEMU).** A counting patch in `MemoryManager::alloc`/`alloc_with_owner`
records, per request: size bucket (8 B steps to 1 KiB, then powers of two), alignment, and owner kind;
and tracks live blocks, peak live blocks and free-list length. It prints through `kprintln` at the
shell prompt and after one run of the integration suite's busy scenarios. Run on both arches.

**Tier 0 cost (host, deterministic enough).** A `#[cfg(test)]` harness drives a bare
`FreeListAllocator` over a 64 MiB buffer: alloc and free time with N live blocks (10, 100, 1 000,
10 000), churn with a seeded xorshift, and fragmentation (`largest_free_block` over `free_bytes`) after
churn. Timings are relative only; bytes and block counts decide.

**Overhead (host, deterministic).** Bytes consumed per request size for Tier 0 today versus the
Tier 1 class table, and internal waste over the K0 distribution (or a synthetic one before K0 exists).
For the user tier: the old and new class tables over the same distribution through `bins.rs`.

**Real boot footprint.** `kstatsd` free bytes at the shell prompt before and after, per arch, where
QEMU is available.

Preconditions for every timing run: `uptime` load against `nproc`, and
`pgrep -f 'cargo test|xtask test|qemu-system'` empty.

## Kernel track

### K0 — baseline and size histogram on the unchanged tree

- Write the harness and run the protocol on `162e69f`: kernel traffic on both arches, Tier 0 host
  costs, fragmentation.
- Use the results to fix `SMALL_MAX`, the class table and `ARENA_PAGES`. If `plans/adaptive-arenas.md`
  has landed by then, run K0 after it: its 32-slot pages are a new allocation shape that may fall above
  or below `SMALL_MAX`.
- Commit `plans/memory-tiers-results.md` (baseline tables, the chosen parameters and why) and nothing
  else. The harness stays uncommitted.
- End with the `retro` skill's step-end mode.

### K1 — Tier 1 slab heap, wired in

- New `kernel/src/memory/slab_heap.rs`: `small_class(&Layout)`, the class table as `const`, the slab
  header, per-class partial lists (intrusive, through headers), the arena list and the
  page-state bitmap in each arena's first page. `SlabHeap::new()` is `const` and allocates nothing.
  `allocate(&mut self, tier0: &mut FreeListAllocator, layout)` and `deallocate(&mut self, ptr, layout)`.
- Growth: no partial slab for the class, take a free page from an arena; no arena page, request an
  arena from Tier 0 (`ARENA_PAGES * PAGE`, align `PAGE`, owner `Kernel`). Tier 0 failure is a null
  return, never a panic.
- K1 never returns pages or arenas; empty slabs stay on their class's list. Trim is K2.
- `MemoryManager`: add the `SlabHeap` field and route in `GlobalAlloc::alloc`/`dealloc` inside the
  existing interrupt guard. `alloc_with_owner` and `deallocate_by_owner` are untouched, so stacks,
  program memory and user blocks behave exactly as before.
- `unsafe` carries `///` of at most 6 lines linking `docs/rationale/kernel-slab-heap.md`, which holds
  the page layout, header-at-end reasoning, alignment rule and the weaker bad-free story. No other
  comments. No `#[allow]`.
- The `unsafe` baseline rises: record it with
  `cargo xtask check-style --update-baseline --allow-increase "<reason>"` in the same commit.
- Unit tests in the module (model: `kernel/src/scheduler/mlfq_strategy.rs`): class mapping boundaries,
  alignment rule and the Tier 0 fallback for over-aligned requests, slot reuse LIFO, a slab filling and
  spilling to a second page, arena growth and Tier 0 exhaustion returning null, double free and
  wrong-class free rejected, `size 0` unchanged; and a randomized alloc/free model (seeded xorshift)
  checking no overlap, alignment, content integrity and that freeing everything restores Tier 0's
  `free_bytes` minus the retained arenas.
- `memory_manager.rs` tests that assert `used == size + ALLOC_HDR` for small layouts change to the
  Tier 1 expectation; larger layouts keep the old assertions.
- Docs commit: `docs/architecture/memory.md` gains the tier model, the routing rule and the new
  bad-free guarantee. The AGENTS.md invariant index line is a proposal and lands only after approval.
- Gate: `cargo xtask check`, then `cargo xtask test` on both arches (small allocations are on every
  boot path), then confirm `[IRQGUARD]` and `BAD-FREE` stay silent in the boot logs.
- End with the `retro` skill's step-end mode.

### K2 — tune, realloc, trim

- Adjust classes and `SMALL_MAX` from K0 if K1's results moved the picture.
- Same-class `realloc` shortcut (the `usrlib` trick), written as an inline fallback and never by
  delegating to the trait default (see `docs/todo/allocator-realloc-tail-extension.md`).
- Return fully-free slab pages to their arena and fully-free arenas to Tier 0, keeping one spare arena
  as hysteresis so a boundary workload does not thrash.
- Tests: trim returns Tier 0 `free_bytes` to the baseline plus one arena; hysteresis; realloc cases.
- Gate: `cargo xtask check`, then `cargo xtask test` on both arches.
- End with the `retro` skill's step-end mode.

### K3 — measure and reassess Tier 0

- Re-run the K0 protocol on the final head with the same harness and host conditions: live blocks at
  the prompt, Tier 0 cost at the new live counts, fragmentation, per-request overhead, boot footprint.
- Append the after tables and the delta to `plans/memory-tiers-results.md`, with a short reading.
- Decide, with the numbers: if Tier 0's O(live) free or its fragmentation is still material for stacks,
  program memory or user blocks, open a follow-up plan from `docs/decisions/page-tier-tlsf-deferred.md`;
  otherwise mark that record decided.
- Gate: `cargo xtask check`; then one `cargo xtask verify` on a quiet host, since the change touches
  boot on both arches.
- End with the `retro` skill's step-end mode.

## User track (independent of the kernel track; may run in parallel)

### U0 — baseline

- Host harness over `bins.rs` computing bytes consumed and internal waste for a seeded size
  distribution (and the K0 distribution once it exists), for the current class table.
- Use the existing `bench` app and the newest record in `benchmarks/` as the speed baseline; add
  mixed-size churn suites only if the existing ladder cannot show the effect.
- Append to `plans/memory-tiers-results.md`. End with the `retro` skill's step-end mode.

### U1 — finer class table

- Replace the power-of-two ladder in `usrlib/src/allocator/bins.rs` with a const table: 16 B steps to
  128 B, then four classes per doubling to 4 KiB. A class is chosen as the smallest whose size is at
  least the request and a multiple of the alignment, so alignment up to the class's power-of-two
  divisor stays free.
- Slab span per class is sized in pages so slab tail waste stays at or below 12.5%; `adopt`'s
  alignment check becomes "span base aligned to the class alignment" instead of "multiple of the chunk".
- Tests: boundaries, alignment, the waste bound over the U0 distribution, adopt/take round trips, the
  existing `bins.rs` suite adapted. Waste against the old table is the acceptance number.
- Gate: `cargo xtask check`; `cargo xtask test` (the `bench_alloc` shape test and the baseline gate run
  there).
- End with the `retro` skill's step-end mode.

### U2 — span headers and O(1) span lookup

- Spans are fetched aligned to their size with the header at the end, so a chunk pointer masks to its
  span header (class, free count, links). This removes the "chunk address does not identify its slab"
  snag from the trim todo.
- Decision to take with numbers at this step: aligned spans (default) versus a small span roster inside
  `Binner`. Aligned spans cost Tier 0 slivers per fetch; fetch several spans per `Alloc` call to
  amortise. Record whichever is chosen in `docs/decisions/user-span-lookup.md`.
- `unsafe` rules as in K1, with `docs/rationale/user-slab-spans.md`.
- Gate: `cargo xtask check`, `cargo xtask test`. End with the `retro` skill's step-end mode.

### U3 — trim

- A fully-free span is unthreaded from its class list and returned with `Syscall::dealloc`, with
  hysteresis (keep one spare span per class) and the checkpoint just before a fetch.
- Delete `docs/todo/allocator-slab-trim.md` in this commit.
- Tests: spike-then-free returns memory; hysteresis; no chunk of a returned span remains on a list.
- Gate: `cargo xtask check`, `cargo xtask test`. End with the `retro` skill's step-end mode.

### U4 — measure and record

- Re-run the U0 protocol on the final head; append the delta to `plans/memory-tiers-results.md`.
- Add the user tier to `docs/architecture/memory.md`.
- Gate: `cargo xtask check`. End with the `retro` skill's step-end mode.

## Risks

- **Weaker bad-free detection in Tier 1.** Mitigated by O(1) checks (magic, class, slot offset, bitmap
  bit) and a `BAD-FREE` log line on failure; wild pointers carrying the magic are not caught. Accepted
  and recorded in K1.
- **Arena pinning.** Long-lived arenas interleave with 64 KiB stacks and group blocks in Tier 0 and
  can pin address space. K2 trim and the spare-arena hysteresis bound it; K3 measures fragmentation.
- **Small machines.** A 64 KiB arena with a 4 KiB header page is a larger step on a low-memory
  x86_32 machine. `ARENA_PAGES` is a constant per arch if needed; K0's boot footprint on x86_32 decides.
- **Cutoff mis-sized.** If most real allocations sit just above `SMALL_MAX`, Tier 1 helps little. K0
  exists to catch this before code is written.
- **Re-entrancy.** Tier 1 growth calls Tier 0 inside one guard. Tier 1 takes `&mut FreeListAllocator`
  as a parameter, so it never re-borrows the manager's `KernelCell`; a unit test pins this.
- **Stats meaning.** `free_bytes` in `kstatsd` counts a retained arena as used. This is expected and
  documented; Tier 1 counters are not exported (that would extend a wire format).
- **Host timing noise.** Decided by bytes and block counts, not nanoseconds.
- **QEMU availability.** K0 kernel traffic, the `cargo xtask test` suites and `verify` need QEMU and
  mtools; the host-only parts (unit tests, Tier 0 host costs, overhead tables) do not.

## Out of scope

- A page allocator under Tier 0 with out-of-band descriptors, page-level ownership, TLSF or buddy, and
  page-granular `Alloc` (`docs/decisions/page-tier-tlsf-deferred.md`; revisited at K3).
- A mid-size band (2 to 64 KiB) allocator; K0 says whether it matters.
- Tier 1 counters through `kstatsd` or `mem` (a wire change; own todo if wanted).
- Per-task caches and lock removal in user space (`docs/todo/allocator-task-local-caches.md`).
- In-place realloc tail extension (`docs/todo/allocator-realloc-tail-extension.md`).
- Any change to `Alloc`/`Dealloc` numbers or semantics, to the `system` crate, or to the 486 feature
  floor. Tier 1 uses `usize` arithmetic only, no atomics and no bit-scan instructions beyond what the
  `check-isa` audit already accepts.

## Friction log

K0 | traffic harness first compile failed: a `static` of `UnsafeCell<Counters>` needs a `Sync` wrapper, kernel build exit 101 | TASK | 1 round
K0 | host cost test first run freed the batch in LIFO order, so the deallocate live-list walk stayed flat and the O(live) scaling was masked; re-run with random-order frees | TASK | 1 round
K0 | plan's "Slab header … at most 64 B" is too tight: class 16 needs a 32 B slot bitmap plus fields; K1 budget is ≤ 96 B, recorded in `plans/memory-tiers-results.md` | PLAN-DEFECT | 0
K1 | `cargo xtask test` first run: `rom_release` failed with `BAD-FREE slab outside-arenas` — the plan routed `GlobalAlloc` but left owner-block frees (`Syscall::Dealloc`, `TaskStack::drop`, the `kernel_box` cells of `LoadElf`/`IpcConnect`/`IpcBind`) going through the routed `dealloc`; fixed with `dealloc_owned` and `take_kernel_box` | PLAN-DEFECT | 1 round + ≈1 h boot-log debug
K1 | `rom_release` free-at-prompt pins failed on Tier 1 arena retention (x86_64 127898→127856, x86_32 128224→128142 KiB); the plan anticipated retention but not that the pins would need re-measuring on both arches | MISSING-FACT | 1 round
K1 | `check-style` first run: doc comment on `take_kernel_box` needed its rationale link and the plain `//` re-pin note in `rom_release.rs` was not allowed; the unsafe baseline was then raised twice, and `--update-baseline` writes one decision file per run, so two records had to be merged by hand | TASK | 1 round
K2 | `check-style` first run: the new trim/realloc functions counted unsafe 395 > baseline 361; consolidated scattered inline `unsafe {}` blocks to 389 and recorded the raise in its own decision file | TASK | 1 round
K2 | realloc unit tests first drafted used wrong class boundaries (treated 32→40 as same-class and 512 as Tier 0; 32 is its own class and 512 is still Tier 1); corrected against `small_class` before running | TASK | 0
K3 | the saved K0 harness no longer applied after K1 (`mod.rs` context) and its `k0traffic` module failed dead-code under `cargo test` until gated `#![cfg(not(test))]`; applied with `--exclude` plus two one-line fixes | ENV-TRAP | 1 round
K3 | serial capture dropped the `[K0] tier1` line in two of three dumps (same truncation quirk K0 saw on `tier0` lines); post-busy arena count inferred from the absence of a 64 KiB step in Tier 0 free | ENV-TRAP | 0

