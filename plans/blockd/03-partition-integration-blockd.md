# Plan 3 — Partition integration: curation inside `blockd`

> **Status:** Executed 2026-11-08, M0–M6 complete on branch `blockd-partition-integration`
> (forked from `main` at `a7ef201`); evidence below. Written 2026-11-08; decisions
> owner-approved in the design discussion of 2026-11-08.
> **Feature:** wire the `rxpart` verdicts (Plan 2) into `blockd`'s register flow so the
> namespace becomes partition-aware: a disk with a valid, supported table exposes
> `ata0d0p0`-style slice devices and hides the whole drive; a disk without one exposes
> the whole drive; a corrupt table exposes the whole drive honestly. Slice arithmetic
> (`base`/`len`) was built in Plan 1 — this plan changes only **how device entries are
> built**. If the two earlier plans were right, the diff footprint is `apps/blockd` plus
> tests and docs; anything larger is a design signal, recorded in the evidence log.
> **Depends on:** Plan 1 (`blockd`, name-keyed contract, `kind/state/base/len` shapes)
> and Plan 2 (`rxpart` verdicts) both complete and green.
> **Method:** facts verified at `78f65ed`; line anchors assume Plan 1 landed (shapes
> cited from `plans/blockd/01` §3).

---

## 0. Decisions (owner-approved 2026-11-08)

1. **The table wins:** valid table with supported entries → the whole-disk device does
   **not exist** in the namespace. Overlap between whole-disk and slice mounts becomes
   impossible *by construction* — no runtime overlap guard anywhere in the stack.
2. **Corrupt table exposes the whole disk** (state `Corrupt` on the device and in
   `List`); rescue path exists because the OS has no table writer yet.
3. **Zero-based partition names from true slots:** `ata{c}d{d}p{slot}`, slot = MBR entry
   index (0..3), holes skipped. Slot numbers match what `ata`'s display prints.
4. **`Register` refresh *is* the staleness story** in v1: re-run `attach ATA` after any
   external table change (harness/QEMU-side image edit, future `fdisk`). No `Rescan`
   opcode.
5. **Extended/logical disks:** per `rxpart` (Plan 2 §3.2) — `blockd` logs the refused
   container types verbatim at register time and exposes whole; the chain-walking
   feature, when it comes, is a parser PR with zero changes to this integration shape.
6. **The vendor `ATA` face remains the sharp knife:** the `ata` CLI can still raw-write
   sector 0 past curation. That is diagnostic territory (AGENTS.md documents it); the
   OS-side table *authoring* tool (`fdisk`-lite, a vendor-face client) stays a separate
   plan with its own busy/mounted handshake.

## 1. Goal & Non-Goals

### Goal
- `attach ATA` discovers drives, reads each drive's sector 0 through the provider, and
  builds the curated namespace via `rxpart::exposure`: whole disks or partition slices,
  with `kind`/`state` bytes filled truthfully.
- Slices behave like first-class devices end-to-end: `format ata0d1p1` →
  `mkdir /data` → `mount ata0d1p1 /data` → persistence across reboot — with **zero**
  changes to `vfsd`, `RemoteBlock`, `rxfs`, or the kernel (the payoff of the device-
  relative design, demonstrated).
- Integration proof on a partitioned image + expose-on-corrupt proof, both arches.

### Non-Goals
- **GPT** (post-LBA48 — unchanged conclusion; `List` already streams 128 entries with no
  wire thought). **`fdisk`-lite.** **Data-plane optimizations** of the hop. **Mount
  visibility in `List`** (mount state stays a `vfsd` fact; see Plan 1 decision record).

## 2. Context Map (verified at `78f65ed`)

- `blockd` (per Plan 1): `DeviceEntry { name, source, description, kind, state,
  provider slot, provider_device, base, len }`; the Read/Write path already applies
  `device_block = base + block` with `base+len` range checks; `Register` already
  tears-down-and-rebuilds per provider (the refresh mechanism). Names currently
  `ata{controller}d{drive}`; discovery probe-scans the numeric face until `NotFound`
  (cap 8).
- `rxpart` (per Plan 2): `parse_mbr(&[u8;512], disk_sectors) -> Table`,
  `exposure(&Table) -> Exposure::{Whole{table}, Partitions(Vec<Slice>)}`,
  `Slice { slot, type_id, start_lba, sector_count }` — total, pure, host-proven.
- Sector-0 read is available on the provider connection: numeric face
  `read_blocks(provider_device, 0, 1)` returns exactly one 512-byte sector.
- `rxfs` device-relative facts (verified): `Geometry::plan(dev.block_count())`
  (`fs/rxfs/src/spec.rs:39-76`); backup superblock written at `total_blocks - 1` and
  probed at the *device's* last block (`fs/rxfs/src/volume.rs:166-230,1291-1312`) —
  both coherent with a slice because `RemoteBlock.block_count()` reports the capped
  `len`. Format zero-writes only its own first 8 blocks (`volume.rs:87-90`) — never a
  partition table.
- Wire needs **nothing**: `kind/state` bytes, 112-byte `List` entries, `NAME_BYTES=24`
  (longest v1 name `ata0d7p3` = 8) were all reserved in Plan 1 §3.1.
- Harness: `tests-integration/src/qemu.rs` writes images as plain byte files
  (`spawn_with_hdd_disk` `:43`, scratch writer `:77-85`) — carving an MBR is a few
  byte writes into the same helper pattern. The x86_64 boot image already carries a real
  MBR (test `ata_client_reads_the_mbr_through_atad` asserts `mbr part3`).
- `attach` app (post Plan 1) renders `List` grouped by `source` — partition rendering
  is display-only work in that app.

## 3. Design

### 3.1 Register flow, partitioned (`apps/blockd`)

For each discovered drive (after the existing probe):

1. `read_blocks(device, 0, 1)` on the provider connection → sector 0 (a provider fault
   during this read is not a device fault: the drive stays out of the namespace with the
   `Device`-honest log, matching today's probe-failure treatment).
2. `rxpart::parse_mbr(&sector, geometry.block_count)` → `Table`;
   `rxpart::exposure(&table)` → decision:
   - `Whole { table }` → one `DeviceEntry`: `kind=Disk`, `state` mapped
     (`NoSignature|Empty → Clean`, `Corrupt → Corrupt`, `Valid → Clean` with an
     "no supported partitions (containers refused: 0x05)"-style note appended to the
     description when in-use containers were skipped); `base=0`, `len=whole`. **The
     drive entry exists.**
   - `Partitions(slices)` → **no drive entry**; one `DeviceEntry` per slice:
     `name = ata{c}d{d}p{slot}`, `kind=Partition`, `state=Clean`,
     `base = start_lba`, `len = sector_count`, description =
     `"{drive} type {:02X}"` (type byte honest, model string kept on… nothing — with the
     drive hidden its model lives in each slice's description line, budget-checked).
3. Register-time logging: per drive, the verdict with the refused container types, so
   `attach ATA` output explains every exposure decision.

No other function changes: `Probe/List/Read/Write/Flush` already consume the table as
built. The `ata` CLI keeps showing raw whole-disk truth (vendor face) — the two views
are documented as complementary, not contradictory.

### 3.2 `attach` rendering (display only)

```
rosx> attach
ata0  ATA
  ata0d0   QEMU HARDDISK 2048x512           clean        (no table → whole)
  ata0d1p0 ata0d1 type 83  65536x512                      (drive hidden: table wins)
  ata0d1p1 ata0d1 type 82  2048x512
rosx> attach ATA          → refresh: same table re-read from sector 0
```

State suffix `corrupt` where applicable. (Exact rendering is the app's; the tree shape
above is the acceptance sketch.)

### 3.3 What must NOT change (the plan's promise, enforced by review)

`protocol/*` (both faces), `apps/vfsd`, `apps/format`, `apps/mount`, `apps/ata`,
`fs/*` (including `rxpart`), `kernel/`, `usrlib/`, boot plans, xtask build logic.
The only non-`blockd`, non-test, non-doc file this plan expects to touch is
`apps/blockd/Cargo.toml` (the `rxpart` dependency). If implementation needs more, stop
and amend the plan before writing it.

## 4. Milestones (tiny steps; each verified before the next)

- **M0** — add `rxpart` dep; unit: verdict → entries mapping over faked provider
  sectors (whole/clean; whole/corrupt; partitions with a hole — `p0` and `p3` present,
  `p1`/`p2` absent; drive-hidden invariant; description budget; container-note). *Gate:*
  `cargo test -p blockd`.
- **M1** — register flow reads sector 0 and applies the mapping (§3.1); refresh
  re-reads. *Gate:* `cargo test -p blockd`; `cargo xtask build` + `x86_32` +
  `check-isa x86_32` green.
- **M2** — `attach` rendering (§3.2). *Gate:* manual `cargo run` with a
  harness-carved image (M3's helper or QEMU invocation by hand).
- **M3** — harness: `spawn_with_partitioned_hdd_disk` — same as
  `spawn_with_hdd_disk` plus hand-carved MBR bytes: slot 1 `type 0x83`,
  `start_lba=2048`, `count=65536`; slot 2 `type 0x82`, count 4096; 55AA. New test file
  `tests-integration/tests/partitioned_hdd_flow.rs`: `attach ATA` tree asserts drive
  hidden + both slices listed → `mount ata0d1 /data` fails `NotFound` (table wins) →
  `format ata0d1p1` → `mkdir /data` → `mount ata0d1p1 /data` → cp/ls/rm roundtrip →
  second boot from the same image → `attach ATA` + `mount ata0d1p1 /data` → data
  persists → `format` while mounted → `Busy`.
- **M4** — expose-on-corrupt test: same harness helper with garbage bytes at 446..510 +
  55AA → bare `attach` lists `ata0d1 … corrupt` whole → `format ata0d1` + mount +
  roundtrip succeed (rescue path proven). `ata0d1p0` → `NotFound`.
- **M5** — refresh proof: within one boot, `ata write` destroys the scratch disk's
  sector 0 (vendor face), re-`attach ATA`, bare `attach` now lists the drive whole —
  Register-as-Rescan demonstrated end-to-end (and documents curation's staleness edge
  with a live witness).
- **M6** — full gates + docs: both-arches integration suite green;
  `vfs_hdd_flow`/`ata_*`/`ipc_vfs` untouched and green (proof the integration is
  `blockd`-local); AGENTS.md "Disk architecture" paragraph extended with the curation
  rule, the vocabulary, and the vendor-face-as-sharp-knife note; this file's Status +
  Evidence updated.

## 5. Verification

- Unit: `cargo test -p blockd` (M0/M1 surface), `cargo test -p rxpart` re-run unchanged
  (Plan 2 green stays green — it must not need edits; needing edits is the design
  signal of §3.3).
- Integration, both arches: new `partitioned_hdd_flow`, new expose-on-corrupt test,
  refresh witness (M5), plus the entire pre-existing suite **unmodified**.
- ISA + zero-warning gates as usual (`check-isa x86_32` must stay green — integer-only
  parser, no new primitives).

## 6. Risks

- **Unaligned slice bases** (63-sector layouts are not 4 KiB aligned): benign — the
  slice unit stays the provider's 512-byte block; `RemoteBlock`'s ratio reads are
  contiguous device blocks, no alignment required. The integration image uses a
  2048-aligned partition deliberately; M3 additionally parses a 63-aligned golden table
  at the `rxpart` level (Plan 2 tests) — a 63-aligned *live* rxfs flow is a named
  follow-up, not a risk to this plan's images.
- **`format ata0d1p1` clobbering its own table?** No: rxfs zeroes its own first 8
  4 KiB blocks *inside the slice* (`volume.rs:87-90`); the table lives in the drive's
  sector 0, outside every slice (`rxpart` refuses entries containing LBA 0).
- **Hidden-drive model string** truncation in slice descriptions — cosmetic; acceptance
  is "type byte + parent name always visible".
- **`ata write` in M5** destroys only the scratch disk (slave, tempdir image); the
  harness's isolation is the safety story, same as `ata_write_roundtrips` today.

## 7. Evidence log convention

Same as Plans 1–2: Status line + trailing `## Evidence` section; additionally records
the final `git diff --stat` of the implementation, checked against §3.3's promise.

---

## Evidence

Executed 2026-11-08 on branch `blockd-partition-integration`, forked from `main` at
`a7ef201`.

| Commit | Milestone |
|---|---|
| `58721a0` | M0+M1 — curation inside `namespace.rs`: sector-0 read through the provider, `rxpart` verdict → device entries, `DriveVerdict` register log; `rxpart` dependency |
| `205c042` | M3+M4+M5 — harness carve helpers, `partitioned_hdd_flow`, `expose_corrupt_hdd`, `blockd_refresh_witness` |
| `be61449` | M6 (docs) — AGENTS.md curation rule, vocabulary, sharp-knife note, corrected footgun and boot-plan claims |

M0 and M1 landed as one commit: the verdict→entries mapping is exercised through
`register()` itself, so there is no intermediate state between the two milestones'
deliverables. M2 needed no code — the existing `attach` rendering (grouped by source,
name + description + `countxsize` + state label) already matches the §3.2 acceptance
shape; the M3 test asserts against the rendered `attach` listing, which replaces a
manual windowed `cargo run` in this headless session.

Gates (commands and results):

- `cargo test -p blockd` — 30 passed (13 new over faked provider sectors: whole/clean,
  signed-empty/clean, corrupt/whole with honest state, holes keep true slots with the
  drive hidden, slice reads forwarded at `start_lba`, description budget, container note,
  long-model note, mixed container refusal, one verdict per drive, sector-0 read fault,
  refresh re-read, verdict rendering).
- `cargo test -p rxpart` — green with zero edits (Plan 2 green stays green; the §3.3
  design signal never fired).
- `cargo xtask build`, `cargo xtask build x86_32` — clean; `cargo xtask check-isa x86_32`
  — kernel and 31 user ELFs 486-clean; zero warnings on every build surface apart from
  the toolchain's own `core` future-incompat report.
- New QEMU proofs, green on both arches: `partitioned_hdd_flow` 40.7 s (x86_64) /
  37.3 s (x86_32); `expose_corrupt_hdd` 21.0 s / 19.7 s; `blockd_refresh_witness`
  20.9 s / 15.8 s.
- Full pre-existing suites, `--no-fail-fast` rerun, both arches green (exit 0 each):
  the first both-arches pass had stopped on two flakes outside the subsystem — x86_64
  `ipc_wake_latency` (missed the 5 s pong window of its wake-ordering witness) and
  x86_32 `partitioned_hdd_flow` (format overran its 60 s budget under suite load) — and
  in the clean rerun both passed in place (`ipc_wake_latency` 17.3 s on x86_64,
  `partitioned_hdd_flow` 37.5 s on x86_32).
  `vfs_hdd_flow`/`ata_*`/`ipc_vfs` untouched and green — the integration is
  `blockd`-local, demonstrated.

Notes, honest:

- **The x86_64 boot drive goes partition-aware too.** The `bootloader` 0.11.17 BiosBoot
  MBR stamps slot 1 (type `0x20`, stage 2) and slot 2 (type `0x0C`, FAT32), which
  `rxpart` judges `Valid → Partitions`: `ata0d0` leaves the namespace and `ata0d0p1`/
  `ata0d0p2` appear. Decision 1 ("the table wins") demands exactly this; no test names
  `ata0d0`, and the AGENTS.md `format ata0d0` footgun line was rewritten with it.
- **Formatted rxfs volumes stay whole.** The superblock payload ends at byte 168, so a
  formatted device's sector 0 carries no `55AA` signature → `NoSignature → whole Clean`;
  the `vfs_hdd_flow` refresh step keeps its drive, and the carved table (drive sector 0)
  sits outside every slice, which the persistence half of `partitioned_hdd_flow` proves
  end-to-end.
- **Two AGENTS.md claims were stale and were corrected** while extending the Disk
  architecture paragraph: `run attach ATA` is not a boot-plan step on either arch (it is
  the `system::boot_plan` test's example only — registration is a manual post-boot
  action, as `vfs_hdd_flow` and these new tests prove), and the blockd entries never
  carried `Available`/`NotFormatted`/`Failed` health — the wire vocabulary is
  `kind`/`state` as of Plan 1.

Diff footprint against the §3.3 promise (final):

```
AGENTS.md                                         |   6 +-
Cargo.lock                                        |   1 +
apps/blockd/Cargo.toml                            |   1 +
apps/blockd/src/main.rs                           |   9 +-
apps/blockd/src/namespace.rs                      | 498 ++++++++++++++++++++--
plans/blockd/03-partition-integration-blockd.md   |   5 +-
tests-integration/src/qemu.rs                     |  28 ++
tests-integration/tests/blockd_refresh_witness.rs |  43 ++
tests-integration/tests/expose_corrupt_hdd.rs     |  53 +++
tests-integration/tests/partitioned_hdd_flow.rs   |  97 +++++
```

`apps/blockd` (plus its `Cargo.toml`), the integration harness and its three new tests,
and docs — nothing in `protocol/*`, `apps/vfsd`, `apps/format`, `apps/mount`, `apps/ata`,
`fs/*` (including `rxpart`), `kernel/`, `usrlib/`, boot plans, or xtask. The promise held.
