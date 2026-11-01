# User-Space Slab Allocator for usrlib — Implementation Plan

> **Status:** Plan (no code written). Written 2026-11-01.
> **Branch base:** `origin/main` at `1123cb8` (#93 bench app + regression gate
> merged). The oracles this plan measures against — the `bench` app, the
> `bench_alloc` integration test, and the `bench_history` gate — all live on
> this base.
> **Feature:** a user-space memory allocator inside `usrlib` that manages a
> program's memory from large slabs fetched from the kernel through the
> existing `Alloc`/`Dealloc` syscalls, amortizing syscall cost across
> thousands of small allocations. The design borrows jemalloc's small-object
> core — size classes, uniform slabs, free lists threaded through free chunks
> — and drops the machinery that makes jemalloc large. **No kernel changes,
> no ABI changes.**
> **Method:** every repo fact below was verified against this working tree
> (paths and line numbers cited; baseline numbers read from the checked-in
> record `benchmarks/bench-x86_64-20261101-113130.txt`).

---

## 1. Goal & Non-Goals

### Goal
- Replace the pass-through `SyscallAllocator` (one syscall pair per
  malloc/free) with a binned slab allocator in `usrlib`, so the hot path for
  small allocations is a few instructions and one kernel round-trip buys
  thousands of allocations.
- Keep the design so small that a reader holds all of it in their head: nine
  size classes, one spinlock, zero per-allocation metadata.
- Prove the win with the existing bench machinery: same workload shapes, new
  machine-readable suites, new baseline record, gate green.
- Close by comparing the post-change numbers against the pre-change baseline
  (§2.8) and reporting the observed results in a small document.

### Non-Goals
- **No kernel or ABI changes.** The existing `Alloc = 7` / `Dealloc = 8`
  numbers already take `(size, align)` (§2.11); slab fetching rides them. No
  new syscall number, no `system` crate change. **Correction found while
  implementing (§2.14):** this holds for the mechanism but not for shipping
  it on by default — a shared per-image cache needs the kernel to reclaim
  blocks per loaded-image group, so that kernel change is the prerequisite
  for the default flip (M4, docs/todo.md). Meanwhile the allocator ships
  behind a usrlib feature and no app's behavior changes.
- **No per-allocation headers or footers.** Rust's `GlobalAlloc` contract
  hands `dealloc` the exact layout given to `alloc` (§2.1); the allocator
  re-derives the size class from the layout and stores nothing beside the
  chunk. This is the single biggest simplification jemalloc cannot enjoy.
- **No memory return to the kernel in this plan.** Freed chunks stay in the
  pool for reuse; exit-time ownership reclaim (§2.5) is the leak safety net
  for single-task programs — shared caches need §2.14's group reclaim.
  A trim path is deferred (docs/todo.md).
- **No task-local caches (tcache).** A global spinlock guards the bins
  (§3.4); per-task caches need a `TaskId` syscall that does not exist yet.
  Deferred (docs/todo.md).
- **No dlmalloc-style coalescing arena in this plan.** Designed as future
  work with explicit composition seams (§5).
- **No kernel-allocator rework.** The kernel's O(live-blocks) dealloc scan
  (§2.2) stays as-is; user-space batching makes it rare, which is the point.

---

## 2. Verified Baseline Facts (2026-11-01, paths)

1. **Every user allocation is two syscalls today.**
   `usrlib/src/runtime.rs:10-23` — `SyscallAllocator` is the
   `#[global_allocator]` under the `runtime` feature; its `alloc`/`dealloc`
   forward straight to `Syscall::alloc`/`Syscall::dealloc`
   (`usrlib/src/syscall.rs:98-104`), one `raw_syscall` each. No caching.
   Note the shape of `dealloc(&self, ptr, layout)`: by the `GlobalAlloc`
   contract, the pointer arrives with the *exact* `Layout` it was allocated
   with, so a user-space allocator can re-derive the size class at free
   time and store no per-chunk metadata.
2. **Kernel allocation is first-fit; kernel deallocation is O(live blocks).**
   `kernel/src/memory/free_list_allocator.rs`: `allocate` linearly scans the
   free list for the first block that fits (102-137); `deallocate` linearly
   scans the **allocated-blocks** linked list to find the block's header
   (184-194) before inserting it back into the free list. Both run under
   interrupt disable/enable in `memory_manager.rs` (89-112, 154-177). The
   bench keeps one live block so the scan looks cheap; a real app with
   thousands of live allocations pays thousands of hops **per free**, with
   interrupts off.
3. **Handler and owner tag.** `kernel/src/syscall.rs:64-73`: `Alloc` builds
   a `Layout`, tags the block `BlockOwner::Task(current)`, and calls
   `MemoryManager::alloc_with_owner`; `Dealloc` reverses by pointer.
4. **Over-aligned requests work in the kernel.** `allocate` honors any
   `align` via a back-reference word (free_list_allocator.rs:127-133);
   proven by `memory_manager.rs:314-324` (`overaligned_alloc_returns_aligned_pointer_and_tracks_used`).
   Slab requests with `align = 4096` ride this path.
5. **Exit-time reclaim is the leak net.** `kernel/src/kernel.rs:265-268` —
   `terminate_and_yield` calls `release_task_allocations(task_handle.pack())`,
   which sweeps every block owned by the task
   (`free_list_allocator.rs:231-250`, `deallocate_by_owner`). A user allocator
   that never frees slabs back still gives all memory back at exit.
   (For a *shared per-image* cache this net is exactly the hazard — fact 14.)
6. **Flat address space.** No per-task page tables exist; the only page-table
   code is the boot table walk (`arch/x86_64/src/cpu.rs:147-160`). Kernel and
   all tasks share one memory space, so interior pointers handed out from a
   user-fetched slab are valid everywhere and the kernel never inspects slab
   contents.
7. **Same-image tasks share statics and are preemptible.**
   `apps/threads/src/main.rs` exercises shared `static` atomics across
   spawned tasks; preemption switches tasks
   (`kernel/src/kernel.rs:255-259`, `apply_pending_preemption`). Any shared
   allocator state in `usrlib` statics can therefore be entered by a sibling
   task after a preemption. There is no TLS and no current-task-id syscall.
8. **Measured cost today — the baseline for the post-change comparison.**
   The checked-in record `benchmarks/bench-x86_64-20261101-113130.txt`
   (v3 format; `host=dev`, `kernel_profile=dev`, QEMU 10.2.1,
   `git_commit=931a982`) is the pre-change baseline. alloc+dealloc pairs,
   `align=8`, best of 3 reps:

   | size (B) | pairs/s | size (B) | pairs/s |
   |---|---|---|---|
   | 16 | 48 188 | 4 097 | 45 714 |
   | 24 | 46 545 | 8 192 | 45 511 |
   | 32 | 46 545 | 16 384 | 44 912 |
   | 64 | 45 511 | 32 768 | 44 800 |
   | 128 | 46 545 | 65 536 | 44 912 |
   | 256 | 45 714 | 131 072 | 44 912 |
   | 512 | 45 714 | 262 144 | 44 281 |
   | 1 000 | 45 714 | 524 288 | 44 912 |
   | 1 024 | 45 714 | 1 048 576 | 43 076 |
   | 2 048 | 45 714 | 2 097 152 | 43 921 |
   | 4 096 | 45 714 | 4 194 304 | 43 389 |

   43.4k–48.2k pairs/s across the whole 16 B…4 MiB ladder — **~21–23 µs per
   pair, flat with size**, confirming fixed syscall + scan overhead
   dominates. Yield floor: 5 333 yields/s. These are exactly the numbers
   `alloc_cached` (M3) must beat on the ≤ 4 096 rows and match on the
   ≥ 4 097 rows.
9. **The bench harness and gate are the oracle.** `apps/bench/src/windows.rs`
   calibrates each phase to a ≥200 ms window, warms up, and reports best of 3
   as machine-readable `bench.<suite>.best` lines.
   `tests-integration/tests/bench_alloc.rs` boots QEMU and asserts the
   `alloc` suite's line shapes; `tests-integration/tests/bench_baseline.rs`
   compares against the newest record in `benchmarks/` (±20 % default
   tolerance, environment-mismatch refusal, `BENCH_RECORD=1` to append a new
   record). `Syscall::alloc`/`Syscall::dealloc` stay raw — `alloc_pairs.rs`
   calls them directly, so they remain the syscall-floor reference even after
   the global allocator changes.
10. **usrlib tests on the host.** `usrlib/src/lib.rs` gates `no_std` and the
    `runtime` module behind `not(test)`; `syscall.rs:347`, `ipc.rs:127`,
    `error.rs:187`, `handle.rs:157` all carry `#[cfg(test)]` modules that run
    on the host. The pattern to follow is exactly what `encode_arguments`
    does: keep the logic pure, keep the syscall edge thin.
11. **ABI is generic enough.** `system/src/syscall_numbers.rs` —
    `Alloc = 7`, `Dealloc = 8`; both already carry arbitrary `(size, align)`.
    Nothing new is needed to fetch a 64 KiB aligned slab.
12. **Repo rules this plan must honor** (AGENTS.md): no comments in code; no
    test-only production surface (every function gets a real caller);
    `kernel/` stays untouched and portable; host unit tests for all new
    logic; commit messages carry no prefix; x86_32 shares `usrlib`, so M2
    verification includes the x86_32 build.
13. **The kernel frees boxed marshalling cells.** `LoadElf`, `IpcConnect`
    and `IpcBind` run `*Box::from_raw` on the pointer they are handed
    (`kernel/src/syscall.rs:76,93,106`): the cell crosses the boundary and
    the kernel becomes its owner. Such a cell must therefore be a
    kernel-owned block, never a slab chunk — with a caching allocator active
    these cells must bypass the cache (`kernel_box` in
    `usrlib/src/syscall.rs`).
14. **Per-task reclaim turns a shared cache into a hazard for a terminating
    sibling.** Fact 5's exit-time reclaim fires per task, but fact 7's
    shared statics span the whole image: when an `exec`'d sibling exits, the
    kernel unmaps the slabs stamped `BlockOwner::Task(that sibling)` while
    chunks it had freed are still threaded on the image's shared free list —
    the next `take` hands out unmapped memory. Reproduced with the slab
    allocator wired as the default global allocator: after the first
    `exec`'d bench suite terminated, `bench alloc_churn` produced total
    console silence (kernel heap corruption). Only bench, stress and threads
    ever call `Syscall::exec`; every other program is single-task, so a
    per-image cache is sound for them. A *shared* cache becomes sound only
    once the kernel reclaims at loaded-image-group granularity (last member
    exits) — the prerequisite for enabling `slab-allocator` by default,
    tracked in docs/todo.md.

---

## 3. Design

### 3.1 Shape

```
layout arrives at UsrAllocator (GlobalAlloc shim)
        │
        ├── size == 0 ────────────────────────────────► class 16 B, never null
        │
        ├── size ≤ SMALL_MAX (4096) && align ≤ class_size(size)
        │        │
        │        ▼
        │   take(class) from bin head           ◄── pop, a few instructions
        │        │ empty:
        │        ▼
        │   one kernel Alloc for a 64 KiB slab (align = SMALL_MAX),
        │   split into uniform chunks, thread them through the bin
        │
        └── otherwise (large or over-aligned)
                 │
                 ▼
        direct Syscall::alloc / Syscall::dealloc passthrough, unchanged
```

Nine classes, one bin per class, one global spinlock, zero per-chunk
metadata. That is the whole mechanism.

### 3.2 Size classes and routing rule

`class_size(size) = max(16, size.next_power_of_two())`:

| class size | 16 | 32 | 64 | 128 | 256 | 512 | 1024 | 2048 | 4096 |
|---|---|---|---|---|---|---|---|---|---|
| chunks per 64 KiB slab | 4096 | 2048 | 1024 | 512 | 256 | 128 | 64 | 32 | 16 |

- **Small path:** `size ≤ 4096 && align ≤ class_size(size)`.
- **Large path:** everything else — one kernel syscall pair per operation,
  today's exact behavior. Over-aligned requests (e.g. size 64 align 4096)
  take it too; §2.4 shows the kernel handles them.
- **Zero-size requests** return a chunk from the 16 B class rather than null
  (jemalloc behavior); `dealloc` with a size-0 layout routes back to the same
  class because routing is a pure function of the layout. This is strictly
  kinder than today's null → `handle_alloc_error` path and stays symmetric.
- Routing symmetry is the load-bearing invariant: the same layout in
  produces the same decision out, so a chunk freed without a header always
  lands in the bin it came from. It rests on the `GlobalAlloc` contract
  (§2.1, §1 non-goals).

Internal fragmentation is bounded at < 2× per chunk with a pow-2 ladder
(requesting one byte over a class boundary doubles the chunk). If a workload
ever shows this to matter, one jemalloc-style intermediate step per octave
(16, 24, 32, 48, 64, …) bounds the worst case to ~33 % and costs a few extra
class columns — deliberately not spent in v1.

### 3.3 Slabs and embedded free lists

- **Slab:** 64 KiB fetched via `Syscall::alloc(65536, 4096)` — one syscall
  buys 16…4096 allocations depending on class.
- **Alignment free-lunch (jemalloc trick):** a slab for class *k* is 4096-
  aligned and every chunk sits at a multiple of the class size, so every
  chunk's alignment ≥ class size ≥ requested size — alignment is guaranteed
  without padding or metadata.
- **Free list:** threaded through the chunks themselves (first word of a free
  chunk = next pointer). No registry, no headers, no bitmap.
- **No slab reuse across classes in v1:** each class grows its own slab
  chain. Worst-case footprint if all nine classes are touched is 9 × 64 KiB
  = 576 KiB. The simplicity is that nothing anywhere has to answer "which
  slab/class does this pointer belong to?" — the layout answers it (§3.2).
- **Memory accounting side effect (good):** the kernel's `used` counter and
  the `mem` app now see arena-level numbers — one 64 KiB block per slab
  instead of thousands of per-object blocks — which is the honest shape of
  what a process holds.

### 3.4 Locking

One `static LOCK: AtomicBool` spinlock guards all bin mutation.

- Acquisition: `compare_exchange(false, true)`; on contention
  `Syscall::task_yield()`, then retry. Release: plain store.
- **Why not a lock-free Treiber stack:** with sibling tasks sharing statics
  (§2.7), the classic pop-vs-refill ABA interleaving is reachable — A loads
  head `X→Y→Z`, B pops `X` then `Y`, pushes `X` back restoring the identical
  `X→Y→Z` shape, A's CAS then succeeds and hands out `X` twice. User space
  cannot disable interrupts or park; there is no per-task storage to key a
  hazard pointer on. A CAS lock with yield-on-contention is a few instructions
  and correct under preemption: the holder always stays runnable (its only
  in-CS syscall is `Yield`, and the scheduler rotates back to it).
- **Contention math:** a critical section is ~10–20 instructions (~20 ns).
  A preemption tick landing inside it has probability ~2e-8 per op at a
  1 kHz tick, so the yield path is statistical noise. The slab-refill path
  holds the lock across one kernel `Alloc` (~20 µs); contending siblings
  yield-rotate meanwhile. Refills happen once per 16–4096 allocations —
  acceptable, and simpler than lock-drop merge dances.
- **Panic-freedom invariant for the allocator itself:** the CS must never
  panic (a panic while holding the lock deadlocks the image). The core uses
  no indexing, no unwrap, no formatting — raw pointer reads and writes
  through checked addresses only. The panic handler (`runtime.rs:63-76`) uses
  `print_raw` (direct syscall, no allocation), so panic reporting stays safe.

### 3.5 Core / backend split (host-testable per §2.10)

- `usrlib/src/allocator/bins.rs` — **pure core, host-tested.** `Binner`:
  nine head pointers, `take(class) -> Option<usize>`, `put(class, addr)`,
  `adopt(class, slab_start)` which splits a slab and pushes its chunks.
  Slab supply is injected as a caller-provided address, so host tests use
  `Vec<u8>` arenas as fake slabs. Every invariant from §3.2/§3.3 is provable
  here: routing table, class boundaries, alignment of every carved chunk,
  carve-then-free-then-retake symmetry, interleavings, exhaustion.
- `usrlib/src/allocator/mod.rs` (`#[cfg(all(feature = "runtime", not(test)))]`)
  — the **shim**: `UsrAllocator: GlobalAlloc` with the static lock, routing
  rule, `Binner` statics, slab fetch, and large passthrough. Thin glue around
  tested logic, mirroring how `runtime.rs` already wraps `print_raw`.
- `usrlib/src/runtime.rs` — swap `SyscallAllocator` for `UsrAllocator`
  (one-line `#[global_allocator]` change at `runtime.rs:22-23`).
- `Syscall::alloc`/`Syscall::dealloc` stay untouched raw syscalls (§2.9) —
  the `bench alloc` suite keeps measuring the syscall floor unchanged.

---

## 4. Milestones

### M1 — Binner core, host-tested only
`usrlib/src/allocator/bins.rs` + `#[cfg(test)]` module. Nothing wired; no
behavior change anywhere.
**Tests:** class routing table (boundaries 15/16/17…4096/4097, the
`align ≤ class_size` rule, zero-size); `adopt` carves exactly
`64KiB / class_size` chunks, every one aligned to class size; take/put
round-trips; interleavings (alloc k, free every other, re-alloc); empty-bin
→ `adopt` → take; independence of the nine classes; no chunk handed out
twice under deterministic mixed sequences (checked against a host-side
occupancy map).
**Verify:** `cargo test -p usrlib`; `cargo xtask build` unaffected.

### M2 — Wire UsrAllocator as an opt-in global allocator
`usrlib/src/allocator/mod.rs` (lock + routing + fetch + slab adoption),
allocator selection in `runtime.rs`. As shipped, the binned allocator is
selected only under a new `slab-allocator` usrlib feature and the default
`#[global_allocator]` stays the syscall passthrough (§2.14): every app's
default behavior is identical to main. Opt-in staging keeps the mechanism,
its host tests and both bare-metal build shapes green, so flipping the
default after the group-ownership fix (M4) is a one-line change.
**Verify:**
- `cargo test -p usrlib` still green (shim is `not(test)`).
- `cargo xtask build` and `cargo xtask build x86_32` (§2.12: x86_32 shares
  usrlib).
- Both usrlib bare-metal configurations compile warning-free:
  `cargo build --target arch/x86_64/rosx-user.json -p usrlib
  --features runtime`, and the same with `--features slab-allocator`.
- QEMU smoke (default config is the unchanged passthrough): shell boots,
  `bench alloc` (raw path unchanged: still reports ~45k pairs/s),
  `snake`/`tetris` idle, IPC roundtrip, stress preemption.
- `cargo test -p tests-integration --test bench_alloc` stays green.

### M3 — Bench suites shipped; gate and report deferred to the flip
New suites in `apps/bench` following the `alloc_pairs.rs` pattern:
- **`alloc_cached`** — same `SIZES` ladder, same pair shape, but through
  `alloc::alloc(layout)` / `alloc::dealloc` (the global allocator). Directly
  comparable to the `alloc` suite: sizes ≤ 4096 expected to leap from ~45k
  pairs/s toward the millions (calibration then simply stops at the harness's
  `MAX_ITERATIONS` cap of 4M), large sizes ≈ unchanged (passthrough).
- **`alloc_churn`** — mixed sizes 16…1024 from a deterministic xorshift
  sequence, steady-state ~50 % live, exercising real bin interleave behavior
  and holding a live working set (the workload that today walks the kernel's
  O(live) dealloc list on every free).

`windows::run_phase` is widened to `&mut dyn FnMut() -> bool` so churn can
carry its live set across phases. Both new suites are excluded from the
default `bench` sweep (`in_default_sweep: false`): while the default
allocator is still the passthrough (M2, §2.14) a sweep would record
passthrough numbers into the slab oracle's history.
`tests-integration/tests/bench_allocator.rs` boots QEMU, runs both suites by
name, asserts their line shapes and asserts no kernel BAD-FREE anywhere in
the transcript; it is `#[ignore]`d until the flip, when it becomes the
soundness gate for the §2.14 fix.

**Closing deliverable — baseline comparison and results report.** Deferred
with the flip: once `slab-allocator` is the default and all four suites ride
the sweep, record a new baseline (`BENCH_RECORD=1 cargo test
-p tests-integration --test bench_baseline -- --ignored`) on the gate host,
commit the record, and write `docs/userspace-allocator-results.md` as
originally specified — per size, the §2.8 baseline pairs/s
(`benchmarks/bench-x86_64-20261101-113130.txt`) vs the cached pairs/s and
the speedup factor, the churn result, the observed `MAX_ITERATIONS`-cap
behavior on the fast sizes, and confirmation that the ≥ 4 097 passthrough
rows stayed within the ±20 % gate. That report remains the plan's last
commit; prose for a reader, not a second machine record.
**Verify:** `bench list` shows four suites; the default sweep and the
committed `benchmarks/` history are untouched; games still play.

### M4 — Deferred (out of scope, tracked in docs/todo.md)
**Prerequisite: task-group memory ownership in the kernel** (§2.14) — stamp
user blocks with the owning loaded-image group and reclaim only when the
last group member exits; then flip the `slab-allocator` default and close
out M3's deferred record and report. After that: task-local caches (tcache)
behind a new `TaskId` syscall; slab trim (returning fully-free slabs to the
kernel); `realloc` fast path.

---

## 5. Future Work: dlmalloc-style Arena (composition path)

Recorded here so the seams are visible while M1–M3 are built. This is the
second allocator personality: instead of fixed classes, **one growable arena
with header-based coalescing** (the dlmalloc shape).

**Sketch.**
- Grow the arena geometrically: request 128 KiB from the kernel, then 256,
  512, … doubling.
- Every chunk carries an 8-byte boundary-tag header (size + previous-free
  bit). Allocation splits from size-binned free lists or bump-carves the
  tail; free coalesces with physically adjacent free neighbors, so long-lived
  mixed workloads keep memory dense.
- Handles every size and alignment with one mechanism, no class ceiling —
  including the mid band (4 KiB…a few hundred KiB) that this plan routes to
  raw syscalls.

**Why not now.** Per-chunk headers duplicate exactly what the `GlobalAlloc`
layout contract gives for free (§2.1); coalescing logic is the classic
bug-magnet (boundary-tag corner cases at arena edges); and the measured win
is already delivered by the bins. Paying dlmalloc's complexity today buys
density no current workload asks for.

**Composition seams (the reason to keep this in the plan, not just in
conversation).** M1's core takes slab addresses from its caller; give that
supply point a name (a slab-source seam) and the two allocators compose in
three useful shapes:
1. **Bins-on-arena** — the bin allocator's slab source becomes an arena
   client instead of raw `Syscall::alloc`: hot small sizes keep popping bins,
   the arena sub-allocates the awkward middle from geometrically grown
   memory, and only huge/over-aligned requests reach the kernel.
2. **Trim that actually recycles** — the arena's coalescer absorbs slabs the
   trim path (todo) hands back, merges them with adjacent free arena memory,
   and re-hands them out; without the arena, trim can only `Dealloc` whole
   slabs back to the kernel.
3. **Comparative oracle** — M3's `alloc_churn` suite becomes the referee:
   bins vs arena vs blend, recorded through the same gate history.

**Trigger conditions to build it:** observed internal-fragmentation pain
(OOM or ballooning `mem` readings in a game/daemon), or mid-size workloads
becoming normal. Until then, the pow-2 ladder plus passthrough is the
right-size answer.
