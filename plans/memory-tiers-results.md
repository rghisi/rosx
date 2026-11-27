# Memory tiers — measurement results

> Companion to `plans/memory-tiers.md`. K0 (baseline on the unchanged tree)
> measured with the disposable harness the plan prescribes: a counting patch
> in `FreeListAllocator::allocate`/`deallocate`/`deallocate_by_owner` with a
> dump on the `kstatsd` memory request, a `#[cfg(test)]` host-cost module
> driving a bare `FreeListAllocator` over a 64 MiB buffer, and a disposable
> integration test driving the shell (`mem`, `bench alloc`,
> `bench alloc_churn`, `mem`). All three pieces live as one patch in the
> scratchpad, never committed. Tree measured: `ccr-067ee909-tqabe6` at
> `5518609` (plan on top of `main@325cf5f`, so after paged arenas #202 and
> the initrd release #201 — the plan directs K0 to run after
> `plans/adaptive-arenas.md`). Host: 4 cores, load 0.06–0.24, no other suite
> live. Byte counts and block counts are the evidence; timings are relative.

## Kernel traffic at the shell prompt (per-owner, cumulative counters)

"PROMPT" is boot + shell startup + one `mem`; "BUSY" is after one run of
`bench alloc` (direct `Syscall::alloc`/`dealloc` pairs, sizes 16 B…4 MiB) and
`bench alloc_churn` (16 B…1 KiB through the user slab tier).

| Owner | arch | requests | bytes | live | peak |
|---|---|---:|---:|---:|---:|
| Kernel | x86_64 | 4 205 | 1 591 378 | 165 | 168 |
| Kernel | x86_32 | 4 044 | 1 476 558 | 165 | 168 |
| Group | x86_64 | 271 | 3 320 895 | 52 | 57 |
| Group | x86_32 | 270 | 3 183 983 | 53 | 57 |
| Stack | x86_64 | 18 | 1 179 648 | 14 | 14 |
| Stack | x86_32 | 18 | 884 736 | 14 | 14 |

Stacks confirm the plan: 64 KiB on x86_64, 48 KiB on x86_32, align 8 — large
Tier 0 blocks, unchanged. Cumulative through BUSY: Kernel 7 082 (x86_64) /
6 843 (x86_32); Group 10 757 073 / 3 895 979; Stack 23 / 23.

Kernel size histogram at PROMPT, x86_64 (x86_32 in parentheses), request counts:

| size bucket | count | class hit (default table) |
|---|---:|---|
| 1..8 | 45 (46) | 16 |
| 9..16 | 2 364 (2 269) | 16 — exact |
| 17..32 | 31 (47) | 24/32 |
| 57..64 | 30 (14) | 64 |
| 65..72 | 0 (20) | 80 (+8…15) |
| 73..80 | 911 (873) | 80 — exact |
| 121..128 | 13 (43) | 128 |
| 177..184 | 0 (86) | 192 (+8…15) |
| 217..232 | 32 (183) | 224/256 |
| 249..256 | 22 (53) | 256 |
| 313..320 | 89 (1) | 320 — exact |
| 361..368 | 105 (0) | 384 (+16) |
| 409..416 | 82 (0) | 448 (+32…39) |
| 441..448 | 30 (0) | 448 — exact |
| 505..512 | 186 (173) | 512 — exact |
| 2 048..4 096 | 189 (174) | — Tier 0 (paged arena pages, #202) |
| 8 192..16 384 | 28 (28) | — Tier 0 |

Every other bucket holds ≤ 19 requests (x86_64: 89..96; x86_32: 1 024..2 048
with 13). Alignment of kernel requests at prompt is ≤ 8 on both arches
(x86_64: 456/2 377/971/401 at align 1/2/4/8; x86_32: 437/2 279/1 328/0). So
every covered class (all have a power-of-two divisor ≥ 8)
honours every observed kernel alignment, and the over-aligned Tier 0
fallback never fires on observed traffic.

The #202 shape K0 was told to check: the 32-slot arena pages land in the
2 048..4 096 bucket — the third-largest kernel bucket (189/174 requests)
and comfortably above `SMALL_MAX`. They stay Tier 0; a mid-size band for
them is not justified at this count.

## Tier 0 host costs (relative, release profile, 64 MiB buffer)

Free in random order with N live 64 B blocks — the O(live) walk:

| live N | ns per free |
|---:|---:|
| 10 | 83 |
| 100 | 154 |
| 1 000 | 1 706 |
| 10 000 | 48 461 |

Linear in live blocks, as the plan states. At today's real counts (≈ 230
live blocks at prompt on both arches) the walk is a few hundred nodes —
sub-microsecond, not yet a live problem. Allocate stays cheap in this
shape (~50 µs per 1 000 allocs at every N) because first-fit re-splits one
coalesced tail.

Churn (seeded xorshift, 200 000 ops, 55/45 alloc/free, sizes 1..512 B
align 8): ≈ 13 000 ops/s with ≈ 20 000 live blocks and ≈ 5 700 free-list
blocks, `largest_free/free_bytes` = 0.994. Interleaving a 64 KiB block
every 100th alloc: same throughput, frag 0.989. At every 20th the heap
saturates: frag falls to 0.066 (largest free block 64 KiB with ≈ 1 MiB
free), large blocks pin the address space, allocations fail — the
plan's pinning risk, reproduced in miniature on the host.

On-device controls from the same boots: `bench alloc` (a direct
`Syscall::alloc`+`dealloc` pair, straight through Tier 0) runs 2 441 ns/op
at size 16 and ≈ 9 000 ns/op at 128 B…512 B; `bench alloc_churn` (served
by the user slab tier) runs 197 ns/op. The existing user-space tier is
12–45× cheaper than the raw Tier 0 path it fronts — the plan's central
argument, measured. The Group-owner BUSY counts (10.7 M requests, x86_64)
are this bench hammering `Alloc` syscalls and stay Tier 0 by the ownership
decision; they are not Tier 1 candidates.

## Bytes per request: Tier 0 today vs Tier 1 default table

Charges at the bucket high end; Tier 1 charge = 4 096 / floor((4 096 − 64)/class),
class chosen as the default-table smallest class ≥ size with
power-of-two-divisor ≥ 8; Tier 0 charge = ALLOC_HDR (32/16) + size rounded
to 8.

| arch | covered subset (≤ 512) | avg Tier 0 | avg Tier 1 | saved |
|---|---:|---:|---:|---:|
| x86_64 | 3 974 requests (94.5%) | 118.2 B | 93.0 B | 25.2 B/req (21%) |
| x86_32 | 3 821 requests (94.5%) | 88.8 B | 79.4 B | 9.4 B/req (11%) |

Through BUSY the covered share rises to 95.6% on both arches. The dominant
bucket (9..16) saves 31.7 B/req on x86_64 (40 B today vs 16.25 B) and
15.7 B/req on x86_32 (32 vs 16.25). Averaged over *all* kernel requests
(including the 2…16 KiB blocks that stay Tier 0 and dominate bytes), the
saving is 5.7% / 2.3% — Tier 1's win is per small request, not per boot.

## Boot footprint (the before numbers for K3)

| arch | used | free | total | free blocks |
|---|---:|---:|---:|---:|
| x86_64 | 1 630 KiB | 127 965 KiB | 129 596 KiB | 78 |
| x86_32 | 1 406 KiB | 128 257 KiB | 129 664 KiB | 93 |

A first Tier 1 arena (64 KiB) is ≈ 4% of used-at-boot and 0.05% of total
memory on both arches — acceptable retention for the 16-page amortisation
the plan chose (≈ 6% sliver cost per arena fetch vs ≈ 25% at 4 pages).

## Parameters decided from these numbers

- **`SMALL_MAX = 512`** — confirmed: covers 94.5% of kernel requests at
  prompt and 95.6% under churn on both arches, and the next real cluster
  (2…4 KiB arena pages) is an order of magnitude above it.
- **Class table: the plan's defaults unchanged**
  (16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320,
  384, 448, 512). The hot buckets (16, 80, 512) hit class boundaries
  exactly; the near misses (65..72 and 177..184 on x86_32, 361..368 and
  409..416 on x86_64) each waste ≤ 39 B on ≤ 2.5% of requests. Adding
  classes 72/232/424 would save ≈ 1.5 KiB over a whole boot — bookkeeping
  noise, declined.
- **`ARENA_PAGES = 16` on both arches** — retention is 0.05% of RAM and
  the sliver amortisation is 3× better than a 4-page arena; the
  "small machines" risk row needs no per-arch constant at 128 MiB.
- **K1 design notes from the data**: the slab header budget must fit the
  slot bitmap — 252 slots (class 16) need 32 B of bitmap alone, so the
  header is ≤ 96 B, not the plan's ≤ 64 B (amortised charge above already
  uses 64 B; at 96 B the class-16 charge is 16.5 B and the savings table
  moves by < 2%). The over-aligned Tier 0 fallback is a safety net: no
  observed kernel request needs align > 8.

## Honest reading

The O(live) free pathology is not hurting today's tree: ≈ 230 live blocks
at prompt, largest free block 99.5% of free bytes, no bad frees, no
fragmentation after the bench run (free blocks 78 → 83, largest
unchanged). What Tier 1 buys is the per-small-request header (a 16 B
request costs 40 B today on x86_64 and 32 B on x86_32; Tier 1 charges
16.25 B — 21%/11% of covered bytes), O(1) allocate
and free instead of first-fit-plus-two-walks as the live count grows, and
the headroom the 10.7 M-op syscall bench and the host saturation run show
is where Tier 0 bends. K3 re-runs this protocol and decides whether Tier 0
still needs more.
