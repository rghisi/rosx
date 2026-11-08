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
