# Multi-sector block reads — `BlockDevice::read_blocks` (B1 of the cluster-chain plan)

> **Status:** Proposed. Written 2026-11-09 at worktree `curious-tiger`, branch
> `block-read-batching`, HEAD `c4339a1` (stacked on `fat-cluster-chain-plan` / PR #152 —
> this work rewrites the same `read_file`/`RemoteWindow` code that #152 rewrote; rebase
> onto `main` when #152 merges).
> **Goal:** collapse the remaining one-sector-per-IPC-hop cost of the engine data path so
> a file load stops paying ~2.5–3 ms per 512 B fetched. Boot to prompt is 2.63 s after
> #152 **[O]** and its remaining ~1.8 s is dominated by exactly these hops.
> **Method:** every claim below is verified against the tree at HEAD (file:line cited);
> costs quoted from #152's measurements. Marked **[O]** = observed on this machine.
> Repo law applies: no comments (code self-explains), unit tests for every step, no
> test-only production surface, commit subjects bare imperative.

---

## 0. Where the time goes after #152

Boot trace at HEAD **[O]**: `0.84 s` root mount, `2.63 s` shell hand-off. The 1.8 s
between is service and shell loads from the FAT volume. One hop `vfsd→blockd→atad`
(IPC round trip + polled PIO) measures ≈2.5–3 ms **[O, #152 §0]**. The FAT window has
density 1 (`unit_bytes = 512 = device block size`), so **one round trip per sector**:

| load | bytes | sectors (round trips) |
|---|---|---|
| `shell` | 58,064 | 114 |
| `/randomd` + `/echo` + `/pong` | ≈ 70,000 | ≈ 137 |
| each shell-launched CLI (resolved per prompt) | per ELF | per 512 B |

`read_file` after #152 reads one sector per `device.read_block` call, and `RemoteWindow`
turns each such call into one `blockd` request of exactly `density = 1` device block.

## 1. The discovery: the stack below `BlockDevice` already speaks "count"

Traced end to end at HEAD; nothing below the trait needs to change:

| layer | evidence | multi? |
|---|---|---|
| wire | `protocol/block/src/wire.rs:51-52` `Read { device, block, count: u16 }` / `Write { …, count, token }`; `wire.rs:3` `MAX_BLOCKS_PER_REQUEST = 128`; `valid_count` 1..=128 | ✅ |
| block client | `protocol/block/src/client.rs:104` `Block::read_blocks` chunks requests at 128 and drains the data object; `manager/client.rs:86` `Manager::read_blocks` same shape over the namespace | ✅ |
| blockd | `apps/blockd/src/namespace.rs:134` `read(name, block, count)` range-checks against `base + len` and forwards `base + block` with the count verbatim; `provider.rs:17,48` carries `count` to the vendor face | ✅ |
| atad | `apps/atad/src/main.rs:357` `read_block_payload` allocates `count · 512` and calls `engine.read_sectors(lba, count, …)`; `taskfile.rs:130` stages **one** `0x20 READ SECTORS` command with the count, then drains DRQ sector by sector | ✅ |
| vfsd window | `apps/vfsd/src/blk.rs:57` `read_block(unit)` issues one `client.read_blocks(name, unit·density, density)` | ✅ — **but one unit per call** |
| trait | `fs/core/src/device.rs:5` `read_block(block, buf)` | ❌ single unit, the only choke point |
| engine loops | `fs/fat/src/file.rs:5` `read_file`; `fs/fat/src/dir.rs` scan loops; `fs/fat/src/volume.rs` `write`/`zero_gap`; `fs/rxfs/src/volume.rs:346` `read` | ❌ one unit per call |

So `rxfs` already fetches 8 device sectors per exchange (4 KiB unit ⇒ density 8); the
FAT window fetches 1, and *both* engines loop one unit per call regardless. The fix is
therefore: let the engine ask for a **run of contiguous units**, and let `RemoteWindow`
serve it as one request. Protocol crates, blockd, atad, kernel: **zero edits**.

## 2. Design

**2.1 Trait additions with defaults.** `fs/core/src/device.rs` gains:

```rust
fn read_blocks(&mut self, first: u64, count: usize, dest: &mut [u8]) -> Result<(), IoError>;
fn write_blocks(&mut self, first: u64, count: usize, data: &[u8]) -> Result<(), IoError>;
```

Contract, expressed by types + tests (no comments): `count ≥ 1`, unit size is
`dest.len() / count` exactly (`read_block` infers it from `buf.len()` the same way),
out-of-range ⇒ `IoError::Device`, no partial-success observable (a mid-run failure is
the device error, exactly like the engine's current loop). Both ship **default
implementations that loop the single-unit method**, so all twelve existing implementors
(`RemoteWindow`; rxfs `MemoryDevice`, `FileDevice`, `SharedDisk`, `Stable`, `Faulty`,
`Volatile`, `CountingDevice`; fat `TestDevice`, `TinyDevice`, `ImageDevice`,
`SharedImage`) keep compiling and behaving unchanged. The default loop is behavior, not
optimization — the specialization is only ever as correct as the loop it replaces.

**2.2 `RemoteWindow` override — the actual win.** Unit `u` occupies device blocks
`[u·density, (u+1)·density)` by construction, so units `[first, first+count)` occupy the
contiguous device range `[first·density, (first+count)·density)`: one
`client.read_blocks(name, first·density, count·density)`. The client self-chunks at 128
device blocks ⇒ one IPC exchange per 128 sectors (FAT window) or per 16 units (rxfs
window). Bounds check against `self.units` before the call.

**2.3 Run detection in the engines.** A run is a maximal span of *contiguous device
sectors* starting at the current read/write position, clamped to the bytes requested.

- `rxfat::read_file` — restructure to **head partial → aligned runs → tail partial**.
  Within a cluster, sectors are contiguous by construction; across clusters the chain
  tells us (`chain[i+1] == chain[i] + 1`) *before* reading, because #152's cursor
  materializes the chain ahead of the bytes. The aligned-run destination in `buf` is
  itself contiguous ⇒ `read_blocks` lands directly into `buf[produced..]` — **no
  temporary buffer on the hot path**. Boot disk geometry is 2 sectors/cluster **[O]**,
  so runs are ≥ 2 and contiguous-cluster files reach tens of sectors per hop; harness
  disks are `-c 1` **[O]** and depend on cross-cluster contiguity alone.
- `rxfat::write` / `zero_gap` — same shape on `data` (direct slice) / zeroed runs
  (stack buffer per chunk, write clamped); the full-sector skip-read optimization from
  #152 is subsumed: an aligned run never needs the read-modify-write.
- `rxfat` directory scans (`read_directory`, `locate_slot`, `find_slot_spot`) — they take
  a sector `Vec` that splits into runs; batching them needs a temporary (the scans
  early-terminate at the first free/matched slot), so **decide by measurement** in
  Step 4; root scans terminate at the first free slot (~3 sectors on the boot disk
  **[O, #152 §7]**), the expected win is small.
- `rxfs::read` (`volume.rs:346`) — within an extent `physical = extent.start + inner` is
  contiguous; same head/run/tail shape. `dev_read`/`dev_write` stay for the metadata
  paths; `MetaCache` (per-block, metadata-only) is deliberately untouched.

**2.4 Caps and boundaries.** No new cap: the 128-device-block wire cap already lives in
the client and is transparent to `BlockDevice` callers. Latent boundary to record, not
change: `atad::read_block_payload` narrows `count` with `as u8` — safe while
`MAX_BLOCKS_PER_REQUEST ≤ 255`; raising the wire cap would have to remember it (the
`as u8` stays honest as long as the client is the only producer of counts).

## 3. Implementation steps (one commit each; x86_32 benchmark after each per §7)

1. **`fs/core` trait.** Add the two methods with looping defaults. Whole workspace must
   compile and every existing test pass — this step must be provably behavior-free.
   Unit test: default loop equals per-unit reads, including a device that errors on the
   k-th block.
2. **`RemoteWindow` override.** Replace the default with the single-request path +
   bounds checks; extend the existing geometry tests (run → device-range math,
   `first + count` overflow refused, count 0 refused).
   Benchmark: x86_32 suite (this is where the hop collapse first becomes *possible*;
   rxfat still reads one unit per call, so expect flat until Step 3).
3. **`rxfat::read_file` runs.** Head/run/tail restructure, direct-slice reads. Give the
   `TestDevice` a `read_blocks` override that logs `(first, count)` and delegates, and
   pin: a contiguous 16-cluster read is one device call for one run; a scattered chain
   falls back per cluster; misaligned head/tail still single-sector.
   Benchmark: this is the boot-load collapse (114 sector hops for `shell` → tens).
4. **`rxfat` write + `zero_gap` runs** (direct-slice `write_blocks`), plus the directory
   measurement call.
5. **`rxfs::read` extent runs** + equivalent counting-device tests.
6. **Plan record + final verification** appended to this file.

Baseline: the §7 **B0** row (this branch, pre-change) is the reference for every step
and for F; #152's final numbers (`fat_partition_flow` ≈ 16.8–17.0 s) agree within noise.
Expect the boot gain mostly in Step 3: the ~1.8 s
load window should trend toward `bytes / 128 sectors-per-hop · 2.75 ms ≈ 0.2–0.5 s`
for well-packed layouts, less on scattered ones; suite totals should drop by roughly the
boot delta × sessions.

## 4. Semantics & risks

1. **Behavior must stay byte-identical.** The default loop *is* the reference: any
   specialized path must match it for every split of every run (property-test style if
   it earns its keep).
2. **Error contract unchanged.** Out-of-range and mid-run failures surface as
   `IoError::Device` from the same call boundary they do today; engines keep mapping
   `Io → FsError::Io`. No partial-run success is observable either way.
3. **Memory.** The hot paths use direct slices — no new large buffers. Temporary buffers
   appear only where a scan needs one (directory batching, if adopted) and reuse the
   `Vec` allocations those scans already make.
4. **The dominant x86_64 path is unaffected by construction**: its root volume is
   `MemoryDevice`, which keeps the default loop; the win there is only on `ata`-mounted
   volumes (the persisting-disk flows).
5. **486 law:** untouched — user ELFs rebuild via `cargo xtask build`; `check-isa` gate.
6. **#152 coupling:** stacked branch. If #152 changes under this one, rebase; the
   read-path restructure in Step 3 assumes #152's `read_file(chain: &[u32], …)` shape
   and the cursor's materialize-ahead.

## 5. Excluded follow-ups

- `-c 4` boot-disk geometry (#152 B2): multiplies the intra-cluster run length —
  orthogonal, measure after Step 3 so the combined effect is attributable.
- Directory-sector batching: decided in Step 4; root scans exit early after ~3 sectors.
- `MetaCache` batching / metadata prefetch: `rxfs` metadata is per-block by design and
  already cached; out of scope.
- atad write-side coalescing beyond the wire cap (128-sector `WRITE SECTORS` is already
  one command per request; nothing left to do below the trait).
- Multi-sector `probe`/`flush` batching across devices: different shape (per-device
  round trips, not per-sector), not this plan.

## 6. Verification gates (after every step; full set at the end)

`cargo test -p fs-core -p rxfat -p rxfs` · `cargo xtask build x86_32` ·
`cargo xtask build` · boot trace (`qemu-system-i386 -cpu 486 -kernel
target/rosx-i486/release/rosx-x86_32` + boot disk at `if=ide,index=0`, `-debugcon
stdio`) · x86_32 integration suite `-- --test-threads=1` **with `ROSX_KERNEL_ELF`
pointed at the release kernel** (AGENTS.md; the dev-profile fallback inflates totals ~3×)
· x86_64 suite once at the end · `cargo xtask check-isa x86_32`.

## 7. Benchmark protocol and run record (x86_32 only)

Timing focuses on the **x86_32 port only** — it is the port that disk-boots through
`rxfat` and where the hop cost lives. x86_64 runs serve as correctness gates, never as
timing evidence.

**B0 before / F after, same commands, same machine, same conditions.** A baseline run
(B0) is taken on the branch head **before any code change** and a final run (F) after
the last step; intermediate runs follow every step, so the curve is cumulative exactly as
in #152 Round 2. One identical recipe each time:

```bash
cargo xtask build x86_32
python3 /tmp/opencode/boot_trace.py qemu-system-i386 -cpu 486 \
    -kernel target/rosx-i486/release/rosx-x86_32 \
    -drive if=ide,index=0,media=disk,format=raw,file=target/rosx-i486-boot-disk.img \
    -debugcon stdio -display none -no-reboot -serial none -parallel none  # → "handing over" timestamp
ROSX_ARCH=x86_32 ROSX_KERNEL_ELF=$PWD/target/rosx-i486/release/rosx-x86_32 \
    cargo test -p tests-integration -- --test-threads=1
# → sum of per-binary `finished in Xs` + the per-binary table (compare per binary,
#   not just the total; run-to-run noise on the total is ±1–2 s)
```

**Timing windows.** This machine is itself a KVM guest (`systemd-detect-virt` → `kvm`)
and the suite's QEMU sessions run under TCG, so wall time tracks raw CPU throughput as
granted by the outer hypervisor. That throughput arrives in windows: on 2026-11-09 the
machine ran ~1.43x slower for ~75-minute stretches, uniformly across every binary —
including ones a filesystem change cannot touch — while every in-guest indicator
(loadavg, CPU/IO PSI, steal, iostat) read quiet. Discipline that survived this:
(1) compare **per binary**, never only totals; (2) a uniform multiplier across binaries
is the machine — confirm by **re-measuring the same commit** (the original B0 rerun
landed within ±0.2 s on 19/20 binaries across a fast/slow/fast sequence, while the
optimized head measured fast and slow in different windows; a mechanism story told
mid-confusion — eager readdir blamed — was disproved only by a same-window head-to-head);
(3) headline claims are **pair-wise inside one window**: B0′ and F′ back-to-back, each
gated on a fast-window `ata_identify` probe before the suite starts.

**Run record — original base (`#152` branch head + plan, pre-#154), 2026-11-09.** The
step curve was measured inside one fast window; totals from the throttled windows that
sandwich it (188.68 / 189.19 for the same optimized tree) are kept only as window evidence:

| run | commit | boot to prompt | x86_32 suite total | `fat_partition_flow` | `vfs_hdd_flow` | notes |
|---|---|---|---|---|---|---|
| **B0** | `c4339a1`+plan (pre-change) | **2.62 s** | **181.12 s** | 16.71 s | 29.52 s | 0 failed; **[O]** 2026-11-09 |
| step 1 | `0fa61a7` | 2.66 s | 180.83 s | 16.74 s | 29.65 s | flat as predicted (defaults unused); one `exit_status` session flaked, green on rerun |
| step 2 | `ef16bf2` | — | — | — | — | not re-run: the override was unreachable until step 3 — flat by construction |
| step 3 | `086b9d1` | **1.77 s** | **132.97 s** | **11.87 s** | **20.45 s** | the hop collapse; 0 failed |
| step 4a | `ac09f09` | 1.78 s | 132.20 s | 11.64 s | 20.60 s | write/zero runs; 0 failed |
| steps 4b+5 | `8fbbf70`+`8a0877f` | 1.75 s | — | 11.75 s | — | head-to-head vs step 4a in one window (`ata_identify` 1.90 = 1.90, `ipc_ping` 2.23 ≈ 2.20, `boot_and_shell` 6.36 ≈ 6.30): the readdir + rxfs-run additions cost ≤2 %, inside noise |

**Run record — merged base (`main` = #152 + #154).** The six commits re-applied onto
`d93353a` are content-identical to the step curve above (engine files byte-identical
to the tested tip `8a0877f`), so the original-base numbers carry the performance
claim. Suite wall-clock on the merged base is **retired as the evidence channel** —
see the timing-window findings; the instrument that replaces it is `apps/fsbench`
(in-guest phases, best-of-N, CPU-control self-normalization).

| run | commit | x86_32 suite | notes |
|---|---|---|---|
| **F′** correctness gate | `eb3e0eb` | 0 failed, 190.92 s | recorded in a slow window — wall time noted, not compared |
| **B0′** / **F′** fsbench head-to-head | `d93353a` vs `eb3e0eb` | deferred | **blocked behind `plans/ipc-volume-wedge.md`**: the first full-volume run of `fsbench` on this branch exposed a latent silent machine death at sustained IPC volume (see that plan for the full forensic record) |

Per-step expected deltas are stated in §3 and were measured on the original base:
boot to prompt 2.62 → ~1.77 s, suite −27 %, steps 4b+5 ≤ 2 % in one window.

## 8. Implementation record

Commits (branch `block-read-batching-v2`, on top of `d93353a`; identical content to
the original `block-read-batching` branch on the #152 base):

| commit | step | content |
|---|---|---|
| `e431212` | 1 | `BlockDevice::read_blocks`/`write_blocks` with looping defaults + `run_unit` contract helper (unit = `dest.len()/count` exactly, `count 0`/uneven/overflow refused); six device-level unit tests pinning the loop, mid-run failure and pre-access refusal |
| `63e1915` | 2 | `RemoteWindow` overrides: pure `device_range` geometry (bounds/overflow before dialing) + one `blockd` request per run; four geometry tests |
| `8d2300d` | 3 | `rxfat::read_file` head/run/tail restructure — `contiguous_run` walks the materialized chain (`chain[i+1] == chain[i]+1`) and the aligned run lands directly in `buf`; five run-shape tests + a read differential sweep (10 offsets × 9 lengths over a holed chain vs the per-sector reference, byte- and error-identical) |
| `76f6d44` | 4a | `rxfat::write_file`/`zero_region` (gap) + `fat::zero_clusters` cluster-run batching with one 128-sector clamped zero buffer; `zero_sectors` shared helper; write differential sweep (8×9), zero differential sweep (6×6), one-run pin |
| `8350edc` | 4b | `read_directory` batched scan (`read_dir_bytes` splits the sector list into contiguous runs); `locate_slot`/`find_slot_spot` kept early-exit single reads; readdir run-pins (whole root = one call, holey chain = one run per span) |
| `eb3e0eb` | 5 | `rxfs::read` extent runs (`dev_read_run`, run = `min(extent.length − inner, remaining blocks)`); three counting tests (packed extent = one run, aligned middle only, one run per extent); `fs/core` `run_unit` reused by the test device |

### Findings

- **The read collapse lands entirely in Step 3.** One FAT file load goes from one hop per
  512 B to one hop per contiguous cluster span; boot-to-prompt 2.62 → ~1.77 s, suite
  −27 %. Steps 4–5 are polish on paths the boot does not walk (writes, readdir, rxfs).
- **Point lookups stay early-exit by principle, full scans batch by measurement.** The
  first 4b draft also batched `locate_slot`/`find_slot_spot`; they were reverted (the
  initial ~7 % boot attribution turned out window-contaminated, see the benchmark
  finding), and the keep/kill decision was re-taken in one clean head-to-head: eager
  whole-directory reads (name resolution reads directories through `find_entry`) measure
  ≤2 % — the root's 0x00 terminator lets the early-exit loop stop after 1–2 sectors,
  while `read_directory` must see every slot anyway, so batching pays there and not for
  point lookups.
- **The machine has timing windows and code cannot invent them.** Every suite run is
  TCG-under-nested-KVM wall time; the outer hypervisor delivered ~1.43x less throughput
  for stretches, invisible from inside (steal stayed 0, iostat quiet, no competing
  processes). The failure modes this produced, recorded so they are not repeated:
  a contaminated attribution (the eager-scan boot delta was measured across a window
  flip), a mis-diagnosis (uniform inflation was blamed on the host *before* the
  same-commit rerun proved the host constant), and a rescue (the same head-to-head that
  killed the mis-diagnosis is what validated 4b+5 as ≤2 %). Detection kit: fast-window
  gate probe before every suite (`ata_identify` ≤ 2.3 s optimized / ≤ 3.2 s baseline),
  per-binary tables, and re-measure-don't-rationalize.
- **Semantics held.** The §4.1 promise paid for itself: the differential sweeps caught the
  one real bug of the rewrite (a mid-cluster-aligned run returning the cluster head
  instead of `cluster_first + used`) before any device-level test could.
- **Default-loop discipline verified:** Steps 1 and 2 measured flat (180.83 vs 181.12 B0),
  exactly as behavior-freedom predicts.
- **Session-timeout flakes are the suite's standing weather**: one 60 s "waiting for
  rose>" stall per handful of full runs (observed once each on `exit_status`,
  `fat_partition_flow`, `vfs_hdd_flow` across the day, on both optimized and baseline
  code), green on isolated rerun. Not attributed to this branch — and `fsbench` later
  caught the severe sibling of that silent-death family (next bullet).
- **`fsbench` earned its keep on day one**: the new in-guest benchmark (`apps/fsbench`;
  run dir `/fsbrun`, phase-gated `fsbench [dir] [phases]`, `bench.<phase>.best` line
  convention, CPU-control normalization) crossed every previous single-connection
  transaction depth and died the machine silently at sustained IPC volume — forensic
  record and next-probe recipe in `plans/ipc-volume-wedge.md`. The integration test
  runs the gated `fsbench /fsbrun 1` (control only) until that latent bug is fixed.
