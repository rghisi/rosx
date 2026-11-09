# rxfat cluster-chain read path — fixing the #149 integration-time regression

> **Status:** Implemented on this branch (Steps 1–5, then Round 2 §7: write-path follow-ups, 2026-11-09).
> Written 2026-11-09 at worktree `curious-tiger`, HEAD `1df0dca`.
> **Goal:** remove the quadratic cluster-chain walk in `rxfat`'s read path so the x86_32
> integration suite returns to (and stays under) its pre-#147 runtime, while keeping every
> session disk-booting from the FAT volume — i.e. fix the engine, not the scenario.
> **Method:** every cost below was measured on this machine (QEMU 10.2.1, KVM, `-cpu 486`)
> or quoted from CI job logs (runs `42580965046`, `42587486297`, `42587490926`); a working
> prototype of Steps 1–2 exists and was measured end-to-end. Marked **[O]** = observed here.
> Repo law applies: no comments (code self-explains), unit tests for every step, no test-only
> production surface, commit subjects bare imperative.

---

## 0. The regression

`#149` (1df0dca, *Slim the x86_32 ROM to the bootstrap set and boot services from disk*) made
every x86_32 integration session boot 4 ELFs (`/randomd`, `/echo`, `/pong`, `/shell`) through
`init` → `vfsd` → `blockd` → `atad` → polled 16-bit ATA PIO, and every shell-launched CLI is
likewise read through `rxfat`. That exposed a quadratic read path in the engine.

**Measured totals** — CI `test result: finished in` sums for the x86_32 integration job:

| run | #151 (pre) | #147 (boot-disk attached, ROM boot) | #149 (services from disk) |
|---|---|---|---|
| suite total | 67.5 s | 79.5 s | **423.4 s** (6.3×) |

Job wall times for the #149 branch PRs: 8m32s / 11m17s / 13m54s (two runs still running when
observed) vs ~2m45s pre-#147. Every session-bearing test binary inflated 5–10× — including
tests that never touch a disk, because the *boot itself* now reads from FAT. **[O]**

**Measured boot phases** **[O]** (QEMU/KVM, kernel + built 64 MiB boot disk, timestamped
debugcon):

| boot to | shipped #149 | prototype (Steps 1–2) | prototype + Step 1 memo alone | pre-#147 (ROM plan) |
|---|---|---|---|---|
| `mount ok … /` | 0.83 s | 0.83 s | 0.83 s | n/a |
| `/randomd` image loaded | 0.79 s | 0.32 s | ~0.13 s | ~0 |
| `/echo` | 0.94 s | 0.33 s | ~0.10 s | ~0 |
| `/pong` | 1.54 s | 0.44 s | ~0.11 s | ~0 |
| `/shell` (58 KB) | **4.89 s** | 0.81 s | ~0.57 s | ~0 |
| `rose>` prompt | **9.7 s** | 3.0 s | 2.5 s | ≈1.2 s |

**Same-machine suite comparisons** **[O]** (`ROSX_ARCH=x86_32`, identical builds, only the
prototype applied):

| binary | shipped | Steps 1–2 | Steps 1–2 + memo |
|---|---|---|---|
| `boot_and_shell` | 48.5 s | 11.9 s | **8.6 s** |
| `exit_status` | 108.7 s | 28.4 s | **19.7 s** |

Full suites with the prototype: x86_32 176.0 s, x86_64 182.1 s, zero failures. **[O]**

Ruled out **[O]**: host-side boot-disk build (`mformat` + `mcopy` of 31 ELFs ≈ 40 ms per
session); the `disk_boot` suite itself (~12 s); x86_64 boot changes (none). Booting *without*
a disk is worse (18.9 s — the `ATA not ready` probe budget + failed-attach retries), so
"skip the master disk" is not an escape hatch.

## 1. Root cause

A FAT file is a linked list of clusters whose links live in the FAT table. Three multipliers
stack, per 4 KiB read chunk of every ELF loaded:

1. **`fs/fat/src/file.rs::read_file` re-walks the whole chain on every call** —
   `fat::cluster_chain(device, boot, start_cluster)` at the top of every read, even though
   `fs/fat/src/volume.rs::open` already walked and cached the chain in `OpenEntry.chain`
   and `read` passes `first_cluster` instead. A `read_whole` (`DATA_READ_CHUNK_BYTES = 4096`,
   `protocol/vfs/src/client.rs`) of `shell` (58,064 B → 114 clusters at 1 sector/cluster)
   costs 15 re-walks × 114 entries ≈ **1,710 chain hops on top of 114 data reads** — 16× more
   FAT navigation than data, growing with file-size².
2. **Each chain hop reads a full 512-byte FAT sector** — `fs/fat/src/fat.rs::next_cluster`
   has no sector memo; 256 FAT16 links fit in one sector, so ~255 of every 256 hops re-read
   the same sector through the whole IPC+PIO path.
3. **Each sector fetch is a `vfsd → blockd → atad` IPC round trip + polled ATA PIO** —
   measured ≈ **2.5–3 ms** per sector **[O]**. This is the per-unit cost the two above get
   multiplied by; reducing it is follow-up §4-B1, out of scope here.

Shell: ≈1,824 sector fetches ≈ 4.9 s **[O]** — the model matches the measurement.

## 2. Design

Make the per-handle chain a **lazy forward cursor** and memoize FAT sectors inside walks.
After this design, across a handle's whole life each FAT link is followed at most once, and
each FAT *sector* is read once per monotone pass.

```
OpenEntry {
    first_cluster, size, epoch,           // epoch = chain_epoch when cursor was (re)seeded
    chain: Vec<u32>,                      // materialized prefix, starts EMPTY
    tail: Option<u32>,                    // next cluster to append; None = End observed
}
```

- `open()` stores the dir entry's `first_cluster` and marks `tail = Some(head)`. **No walk.**
- `materialize(index)` (internal): while `chain.len() <= index && tail.is_some()`, append the
  tail cluster and follow one link via the two-slot FAT memo; End sets `tail = None`. Reading
  an index past the materialized end after End → `Corrupt` (same shape as today's
  `chain.get(...).ok_or(Corrupt)`).
- `read(offset)`: `materialize(offset / cluster_bytes)`, then index into the prefix. A corrupt
  link surfaces on the read that touches it — reads inside a healthy prefix of a partially
  corrupt chain **succeed**. This is a deliberate semantics change (§5).
- `write` extends the cursor locally: `allocate_chain`/`append_chain` return the newly linked
  cluster vector (they already build it while allocating — `free[index]` loop), and the cursor
  appends it. The current `chain = fat::cluster_chain(...)` full re-walk after every extending
  write (volume.rs:393) disappears.
- **Invalidation: one global counter.** `FileSystem.chain_epoch: u64`, incremented by every
  FAT-entry mutation. `write_fat_entry` is the single choke point — all FAT-entry writers
  funnel through it (`allocate_chain`, `append_chain`, `free_chain`). FAT data-area zeroing
  (fat.rs ~304/332) and FSINFO free-count writes touch no links → no bump. A handle whose
  `epoch != chain_epoch` re-seeds its cursor (empty prefix, `tail = Some(first_cluster)`)
  before materializing — lazy, so the cost is paid only by handles that read again after a
  FAT write. Own extends re-stamp the handle, so appends never re-walk. Rationale for a global
  (not per-file) counter: FAT mutations are rare in this engine, one `u64` + one comparison
  per read is the honest cost, and it restores the freshness the old per-read re-walk gave
  (e.g. a second open handle sees another handle's extension instead of a stale chain).
  Out-of-band sector writers (vendor `ata` CLI, host image swap) stay out of contract, as
  today, guarded by the mount-table Busy invariant at the daemon edge.
- **No LRU anywhere.** The FAT memo is call-scoped: a monotone walk occupies at most two FAT
  sectors (current + next across a boundary), so a 2-slot "replace the older" memo is
  provably as good as any policy; it dies with the call — nothing to invalidate. The cursor's
  lifetime is the handle: `close()` drops the slot and the prefix with it — a chain that
  outlives its handle would need epoch checks *and* eviction *and* generation logic to save
  ≈1 sector read per reopen (memo-cheap, see table). Reopen cost after this design **[O,
  modeled, FAT16, 512 B clusters]**: `randomd` 40 clusters → 1 FAT sector read; `shell` 114 →
  1; 1 MB file 2048 → 8.

## 3. Implementation steps

Each step lands green on `cargo test -p rxfat` plus the gates at the end.

**Step 1 — FAT-sector memo in `cluster_chain` (`fs/fat/src/fat.rs`).**
`cluster_chain` keeps a `Option<(u64, [u8; SECTOR_BYTES])>` local memo and follows links
through a memoized hopper; `next_cluster` stays as-is for single-step callers
(`append_chain`). Call-scoped ⇒ no invalidation, no API change. Prototype: +38 lines.
Expected: chain walks cost ⌈n/256⌉ (FAT16) / ⌈n/128⌉ (FAT32) reads instead of n.

**Step 2 — `read_file` takes a chain slice (`fs/fat/src/file.rs`, `volume.rs::read`).**
`read_file(device, boot, chain: &[u32], size, offset, buf)` — the `cluster_chain` call and
the `crate::fat` import disappear from `file.rs`; `read` clones the handle's chain and passes
it. This is the interim fix (measured: boot 9.7 s → 3.0 s, `exit_status` → 28.4 s); the
prototype for Steps 1–2 is 54 lines across `file.rs`/`fat.rs`/`volume.rs` and already passes
both integration suites. Ship alone or merged into Step 3 at the maintainer's preference.

**Step 3 — lazy forward cursor (`volume.rs`, with `fat.rs` helpers).**
Replace `OpenEntry { chain }` eager field with the cursor of §2: empty prefix + `tail` +
`epoch`; add `materialize`; `open` drops its `data_chain` walk; `read` materializes; `write`
extends via the returned vectors of `allocate_chain`/`append_chain` (signature change stays
`pub(crate)`-within-crate: those helpers return `Vec<u32>` alongside the head/tail). The two
`dir::directory_sectors`/`allocate_slot` walks stay as-is — they are short chains already
cheap under the Step 1 memo.

**Step 4 — chain epoch (`volume.rs`).**
`chain_epoch: u64` on `FileSystem`; bump at the four FAT-mutating call sites in `volume.rs`
(`append_chain` ×2, `allocate_chain`, `free_chain` ×2 — enumerate in review; or thread the
counter into `fat.rs` writers if a cleaner choke reads better). Read/write re-seed stale
cursors. No counter exists on read-only mounts' paths — reads alone never re-seed.

**Step 5 — tests (`#[cfg(test)]` in each touched file).**
- A read-counting `TestDevice` variant (extend `fat::fixtures`): full sequential read of a
  k-cluster file costs exactly k data reads + ⌈k/256⌉ FAT sector reads and **zero** open-time
  chain reads; two handles read the same file → second reopen re-reads its links only.
- Reshape `chain_corruption_surfaces_from_reads` to the lazy semantics: a hole mid-chain fails
  the first read that touches it, and reads below the hole succeed. *(With the prototype's
  eager-at-open variant this test needed re-purposing to `open`; with the lazy cursor it
  regains read-time semantics naturally.)*
- Extension freshness: handle A grows the file; handle B's next read (after epoch re-seed)
  walks and succeeds with the new links — proving the epoch replaces the old per-read re-walk.
- Keep `sizes_beyond_the_cluster_chain_are_corrupt`, straddle/offset cases green.

**Verification gates (every step):**
`cargo test -p rxfat` · `cargo test -p kernel -- --test-threads=1` (untouched, cheap) ·
`cargo xtask build x86_32` + `cargo xtask build` · boot trace
(`ROSX_ARCH=x86_32 … cargo test -p tests-integration --test measure_boot -- --ignored`) ·
both integration suites `-- --test-threads=1` with `ROSX_KERNEL_ELF` pointed at the
release kernel (unsetting it makes `kernel_build` fall back to a **dev-profile** kernel and
inflates the totals ~3x - a measured trap, not a regression) · `cargo xtask check-isa
x86_32` (rxfat rides into user ELFs).

**Expected landing point** (prototype-measured): boot ≈2.5–3.0 s vs shipped 9.7 s;
`boot_and_shell` ≈8.6–11.9 s vs 48.5; `exit_status` ≈19.7–28.4 s vs 108.7; CI x86_32 suite
sum returns under ~110 s (67.5 s baseline + unavoidable disk-boot scenario cost + `disk_boot`),
i.e. the 6.3× regression becomes scenario cost of ≈2× baseline boot — the price of #149's
disk-boot design, paid once per session at ~2.5 s instead of ~9.7 s.

## 4. Follow-ups (measured, deliberately not in this plan)

- **B1 — fewer hops:** the dominant remaining cost is ≈2.5–3 ms per 512 B sector through
  `vfsd→blockd→atad`. `block-protocol`'s `Read` already carries a transfer count and `atad`
  already reads multi-sector; add `BlockDevice::read_blocks(unit, count)` (default
  single-block loop) and have `rxfat` fetch a whole cluster run in one call. With `-c 4`
  geometry, ELF loads drop to ≈4 sectors/hop. Biggest next lever; needs its own design pass
  because it changes the `fs/core` treaty.
- **B2 — boot-disk geometry:** switch harness + `xtask` boot disks from `mformat -c 1` to
  `-c 4`: measured standalone 9.7 s → 5.9 s without any code change; shorter chains benefit
  the write path and shrink the FAT. Trap: **keep the FAT16 floor** — a 16 MiB volume at
  `-c 16` falls under 4,085 clusters and mtools silently produces FAT12, which `rxfat`
  refuses and the boot plan answers with NotFound-from-ROM (observed **[O]**).
- **B3 — small LRU directory-sector cache** (byte-bounded, epoch-invalidated): every
  `lookup`+`open` pair re-reads the 14 root-dir sectors (~150 ms per launch). Measure after
  this plan; refinement, not the fix.

## 5. Semantics changes & risks

1. **Corruption detection moves to touch time.** Eager walk (today) fails `open` on any dead
   link anywhere; lazy fails the read that reaches it. Rationale: errors surface where they
   matter; `stat` and partial reads of long files no longer pay for (or trip on) untouched
   tails. The `check`-style full validation belongs to fsck territory, and `rxfat` has none
   today either way. **[O verified]** `a_chain_hole_surfaces_on_the_read_that_touches_it`.
2. **Chain freshness becomes the epoch's, and it means *resolve against the current FAT*.**
   The old per-read re-walk observed the FAT as it is now on every read; the re-seeded cursor
   matches that exactly. One correction to the intuition in this plan's first draft: for a
   remove-plus-realloc under an open handle, "faithful to the current FAT" is *not*
   `Corrupt` - if the successor's chain is valid at the offsets the stale reader asks for,
   both the old engine and the epoch re-seed serve the successor's bytes (the file the
   handle opened no longer exists; there is no correct answer at that boundary). The epoch's
   job is refusing to follow *dead links* the old walk would have refused - diverging from a
   hypothetical epoch-less cache in both directions. **[O verified]**
   `a_stale_cursor_resolves_against_the_current_fat`.
3. **`OpenEntry.size`/`first_cluster` remain open-time snapshots** (unchanged hazard today);
   the epoch does not pretend to fix snapshot staleness of the dir entry, only of the chain.
4. Step 2 was landed as its own commit but is transient by design - the cursor (Step 3) and
   the epoch (Step 4) ship in the same branch, so the interim stale-second-handle gap of the
   clone-and-read shape never reaches `main`.

## 6. Implementation record (this branch)

Four commits, all measured on this machine (QEMU/KVM, `-cpu 486`) after the final tree:

| metric | shipped #149 | implemented |
|---|---|---|
| boot to `rose>` | 9.7 s | **2.6 s** **[O]** |
| `cargo test -p rxfat` | 121 + 5 | 125 + 5, 0 failed **[O]** |
| `cargo xtask test --skip-integration` | - | 47/47 binaries green **[O]** |
| x86_32 integration suite (local, release kernel) | - | 183.2 s, 69 passed, 0 failed **[O]** |
| x86_64 integration suite (local, release kernel) | - | 184.7 s, 69 passed, 0 failed **[O]** |

- `aded5cc` Step 1: `next_cluster` takes a call-scoped `FatSectorMemo`; `cluster_chain`
  walks with it; `TestDevice` gains `read_log`; `a_long_chain_walk_reads_each_fat_sector_once`
  pins 300 links to 2 sector reads.
- `09708be` Step 2 (transient): `read_file` takes `chain: &[u32]`; `read` passes the
  open-cached chain; `chain_corruption_surfaces_from_reads` becomes
  `a_chain_shorter_than_the_size_is_corrupt` (hole-in-FAT is no longer `read_file`'s view).
- `9626d4d` Steps 3-4: `OpenEntry { chain: prefix, tail, epoch }`, `chain_epoch` on
  `FileSystem` bumped at the five FAT-link mutation sites (`allocate_slot` append, `write`
  grow, `mkdir` allocate, `remove`/`rmdir` free), `open` walks nothing, `write` extends the
  cursor from the vectors `allocate_chain`/`append_chain` now return (their full re-walk is
  gone), `OpenEntry.parent` dropped (dead once `write` stopped cloning the entry).
- `faab9bb` Step 5: `a_chain_hole_surfaces_on_the_read_that_touches_it`,
  `opening_and_reading_reach_the_fat_only_for_the_clusters_served` (open: 0 FAT reads;
  whole-file read: 1 FAT sector + 2 data sectors; cached-prefix read: 0 FAT reads),
  `a_stale_cursor_resolves_against_the_current_fat`.

Reproduction recipe: boot `target/rosx-i486/release/rosx-x86_32` with
`target/rosx-i486-boot-disk.img` at `if=ide,index=0`, `-cpu 486`, timestamped
`-debugcon stdio`; model check: `shell` 58,064 B gives 114 clusters x 16 chain-walking calls
+ 114 data = 1,938 sector fetches (4.9 s at 2.5 ms/fetch), against 115 fetches after this
branch.

## 7. Round 2 — write-path follow-ups (same branch)

Steps 1–5 fixed the *read* path; the same investigation ranked four write/allocation
follow-ups and all four landed, one commit each, benchmarked (x86_32 integration suite +
boot trace) after each to make the cumulative curve visible:

| commit | change | mechanism |
|---|---|---|
| `3d9ec6e` | zero only gap sectors | allocation stopped pre-zeroing every cluster (every created byte was written twice across the IPC+PIO path); `write` zero-fills `[old_eof, offset)` gaps and read-modify-write keeps the below-EOF-valid / above-EOF-zero invariant; directory allocations (`mkdir`, dir-chain append) still take clean clusters via `fat::zero_clusters` |
| `6eb48d8` | `FatEditor` sector-caching writer | `allocate_chain`/`append_chain`/`free_chain` patch FAT entries through one dirty-sector buffer per copy: a 300-link path is 4 reads + 4 writes instead of 600 + 600 (pinned by test) |
| `4f30679` | allocation watermark | `scan_free_clusters` starts at `alloc_hint` (advanced to `top+1` on allocate, lowered to the lowest freed cluster on free - max-allocate/min-free keeps "nothing free below the hint" provably true, so a full volume never reports a false `NoSpace`) |
| `35acc3d` | one-walk unlink | `free_chain` returns `FreedChain { clusters, lowest }`; `remove`/`rmdir` dropped their pre-count `data_chain` walk (helper deleted - no callers left) |

Measured **[O]** (x86_32 suite, release kernel, per-binary extraction; boot flat each step -
this round targets writes and the boot path is reads):

| step | suite total (s) | `fat_partition_flow` (s) |
|---|---|---|
| after §6 | 179.9 | 18.65 |
| gap-sector zeroing | 179.3 | 18.35 |
| `FatEditor` coalescing | 178.3 | **17.01** |
| allocation watermark | 177.7 | 16.70 |
| one-walk unlink | 177.8 | 16.82 |

The suite is boot/read-dominated, so the total moves ~1%; the honest summary is that the
**only binary exercising the FAT write path (`fat_partition_flow`) drops ~10%**, and the
gaps not visible here (creation double-write, per-entry FAT traffic, allocation rescans)
are pinned by unit tests on counting devices (`read_log`/`write_log`), not by suite timing.

Semantics notes: allocation leaving dirty clusters is internal (the below-EOF-valid
invariant is preserved by `write`); `remove`/`rmdir` of a *corrupt-chain* file now erase
the slot first and report `Corrupt` from `free_chain` (freeing nothing, leaking the
clusters - leak-safe, same crash posture as before, no pre-walk guard).

Gates after Round 2: rxfat 130 + 5 mtools; `cargo xtask test --skip-integration` 47/47;
x86_32 suite 0 failed; x86_64 suite 0 failed (181.5 s); `check-isa x86_32` clean; boot
2.63 s. B1 (multi-sector `BlockDevice`) remains the next big lever; B3 is weakened by the
finding that root scans terminate at the first free slot (~3 sectors on the boot disk, not
32).
