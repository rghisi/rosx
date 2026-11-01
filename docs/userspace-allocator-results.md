# User-Space Binned Slab Allocator — Measured Results

**Measured:** 2026-11-01 on the gate host (`host=dev`, 4 CPUs, KVM available: no,
`kernel_profile=dev`, QEMU 10.2.1, `git_commit=258c839`).
**Baseline:** `benchmarks/bench-x86_64-20261101-113130.txt` (v3 record, `git_commit=931a982`),
the pass-through allocator, reproduced in plan `plans/userspace-allocator.md` §2.8.
**New record:** `benchmarks/bench-x86_64-20261101-161226.txt` — the first four-suite
`bench` sweep with `UsrAllocator` as the global allocator.

## The win: binned sizes (16 B…4096 B)

`alloc_cached` runs the same size ladder as `alloc`, but through the global
allocator. Sizes ≤ 4096 B now ride the binned slab pool; the pair cost drops from
~21–22 µs to ~0.21 µs:

| size (B) | baseline pairs/s | slab pairs/s | speedup |
|---|---|---|---|
| 16 | 48 188 | 4 660 337 | 96.7× |
| 24 | 46 545 | 4 660 337 | 100.1× |
| 32 | 46 545 | 4 660 337 | 100.1× |
| 64 | 45 511 | 4 681 142 | 102.9× |
| 128 | 46 545 | 4 660 337 | 100.1× |
| 256 | 45 714 | 4 660 337 | 102.0× |
| 512 | 45 714 | 4 660 337 | 102.0× |
| 1 000 | 45 714 | 4 559 026 | 99.7× |
| 1 024 | 45 714 | 4 681 142 | 102.4× |
| 2 048 | 45 714 | 4 660 337 | 102.0× |
| 4 096 | 45 714 | 4 660 337 | 102.0× |

Flat ~4.6–4.7 M pairs/s across every class: the fixed syscall pair (and the
kernel's O(live-blocks) dealloc scan) is gone, leaving the lock-protected bin
pop/push. The kernel `Alloc` now runs once per slab — every 4096 allocations in
the 16 B class, every 16 in the 4 KiB class.

## The floor holds: passthrough sizes (4097 B…4 MiB)

Requests over `SMALL_MAX = 4096` route straight to the kernel, so these rows
should be noise-identical to the baseline. They are — every delta is inside the
±20 % gate (worst: −6.6 %):

| size (B) | baseline pairs/s | slab pairs/s | delta |
|---|---|---|---|
| 4 097 | 45 714 | 43 389 | −5.1 % |
| 8 192 | 45 511 | 44 137 | −3.0 % |
| 16 384 | 44 912 | 42 264 | −5.9 % |
| 32 768 | 44 800 | 43 076 | −3.9 % |
| 65 536 | 44 912 | 43 389 | −3.4 % |
| 131 072 | 44 912 | 42 010 | −6.5 % |
| 262 144 | 44 281 | 42 264 | −4.6 % |
| 524 288 | 44 912 | 41 967 | −6.6 % |
| 1 048 576 | 43 076 | 42 010 | −2.5 % |
| 2 097 152 | 43 921 | 42 264 | −3.8 % |
| 4 194 304 | 43 389 | 41 290 | −4.8 % |

The raw `alloc` suite (direct `Syscall::alloc`, never binned) also repeats the
baseline (47 407 vs 48 188 pairs/s at 16 B), confirming the syscall floor itself
is unchanged and the speedup is the allocator, not the environment.

## The workload that matters: `alloc_churn`

`alloc_churn` mixes 16 B…1 KiB allocations against a live working set held in
the 256–1024-block band — the shape that made the kernel's dealloc scan walk
thousands of headers per free with interrupts off. The pass-through allocator
ran ~45.7 k pairs/s at these sizes; the slab allocator sustains
**5 994 146 single alloc-or-free ops/s** (≈ 3.0 M pair-equivalent pairs/s, ~65×
the baseline pair rate) with the live set resident the whole time. Bins absorb
the churn directly: freed blocks return to their class list and are re-handed
out without touching the kernel.

## Calibration-cap behavior

The harness ends a measurement when the window reaches 200 ms
(`MIN_WINDOW_MS`) or 4 M iterations (`MAX_ITERATIONS`) hit first. At the
measured ~4.66 M pairs/s the 200 ms floor arrives at ~0.93 M iterations, so
every binned size still measured a genuine time window — the cap was not
binding with a comfortable 4× margin. No fast size degenerated into a
cap-truncated count.

## Verdict

Aggregate: **~100× faster allocation across the entire binned range**
(16 B…4096 B), ~65× on the mixed-size live-working-set churn, no regression on
passthrough sizes (all within the gate), raw syscall floor unchanged, and the
kernel's BAD-FREE detector silent through the full multi-suite sweep (the
`bench_allocator` gate now runs un-ignored on every integration pass).
Soundness rests on the kernel's loaded-image task groups: reclaim fires only
when a group's last member exits, so shared per-image bins can never dangle.
