# Plan 2 — Partition core: `fs/partition` (`rxpart`)

> **Status:** Implemented on branch `blockd-partition-core` (2026-11-08) — see
> `## Evidence`. Written 2026-11-08; decisions owner-approved in the
> design discussion of 2026-11-08.
> **Feature:** a standalone `no_std`, zero-dependency crate `fs/partition` (package
> `rxpart`) that parses MBR master boot records and applies the **curation rule** —
> *a disk is addressed whole only when it has no (readable, supported) partition table*.
> Pure data in / verdicts out: it never touches a device. Fully tested on the host in
> isolation before anything integrates it. **Plan 3** wires it into `blockd`; by design
> this crate knows nothing about services, names, or IPC.
> **Depends on:** nothing in the tree (it can be built any time); its consumer, Plan 3,
> depends on Plan 1.
> **Method:** rules below restate the owner decisions of 2026-11-08 (expose-on-corrupt,
> primary-only, container-type refusal, zero-based slots). Facts verified at `78f65ed`.

---

## 0. Decisions (owner-approved 2026-11-08)

1. **Scope: MBR primary partitions only.** GPT is deferred behind LBA48 (the driver caps
   at LBA28/~128 GiB, making GPT's raison d'être unreachable); extended/logical
   containers are refused honestly, not half-supported.
2. **Curation rule (the product of this crate):** valid table with ≥1 supported entry →
   partitions; no signature, empty table, or corrupt table → whole disk. **Corrupt
   exposes the disk** (operator must not be bricked when the OS has no table-writing
   tool yet), with the state reported honestly.
3. **Slot-indexed naming support:** parse keeps entries at their MBR slot indices
   (`0..3`); Plan 3 derives `ata0d0p0`-style names from non-empty slots — holes are
   skipped, indices stay true to the on-disk table.
4. **CHS fields are ignored entirely** (LBA-only driver, LBA-only verdicts). The boot
   indicator byte is parsed (it is part of the entry) but has no meaning to us in v1 —
   no booting from partitions in scope.
5. **The crate is mechanism, not treaty:** it lives in the `fs/` theme (on-disk
   treaties), not `protocol/` (wire treaties) — consistent with `fs-core`/`rxfs`.
6. Package name **`rxpart`**, directory `fs/partition` (owner's directory choice; the
   package follows the `rxfs`/`rxfat` naming pattern).

## 1. Goal & Non-Goals

### Goal
- `rxpart` parses a 512-byte master boot record plus the disk's sector count and returns
  a typed `Table` verdict and an `Exposure` decision — total, infallible (no `Result`
  for malformed input; malformed means `Corrupt`, not failure); the parse itself is
  allocation-free.
- The curation rule is a **pure function in this crate**, so partition *policy* is
  host-testable without QEMU, disks, or IPC — the deepest argument for the `blockd`
  topology lands here.
- Host unit tests: golden byte arrays, every boundary, and a deterministic no-panic
  sweep over seeded pseudo-random sectors.

### Non-Goals
- **GPT** (post-LBA48; a parser-crate PR then — `List` streams 128 slices with zero wire
  thought per Plan 1's shape-stability decision).
- **Extended/logical resolution** (container types are *recognized and refused*; chain
  walking is a named future feature).
- Any device IO, naming (`ata0d1p0` strings), IPC, or `fs-core` dependency.
- Table *writing* (the `fdisk`-lite tool plan; it will consume this crate's types and
  add encoding).
- Filesystem-type semantics: `rxpart` reports type bytes, it does not identify
  filesystems (no `blkid`).

## 2. Context Map (verified at `78f65ed`)

- `xtask` integration images already stamp the 55AA signature
  (`xtask/src/main.rs:13-14,118-119`; `tests-integration/src/qemu.rs`
  `attach_multiboot_master_disk` `:287-301`), and the `ata` CLI already decodes MBR
  entries for display (`apps/ata/src/main.rs:10-16,79-103`: `MBR_PARTITION_FIRST=446`,
  `MBR_PARTITION_BYTES=16`, `MBR_PARTITION_COUNT=4`, `MBR_SIGNATURE_OFFSET=510`,
  entry bytes `[0] boot, [4] type, [8..12] lba, [12..16] sectors`). `rxpart` is the
  real parser that display anticipated; the CLI may later adopt it — not in this plan.
- rxfs was designed device-relative (`plans/rxfs-filesystem-crate.md:43,48`): a slice
  fed through the `BlockDevice` seam needs nothing from this crate directly — this
  crate's only reader is `blockd` (Plan 3).
- Workspace: `fs/*` crates are explicit members (`Cargo.toml:5-8`) — `fs/partition`
  joins that list and `default-members` (like `fs/ramfs`), so `cargo test` and
  `cargo xtask test` pick it up with no xtask change.
- House style for such crates: zero third-party deps, `#![no_std]` + `extern crate alloc`
  (`rxpart` uses `alloc` only for `Exposure::Partitions` — once per verdict, never on a
  hot path), `#[cfg(test)]` modules in each file.

## 3. Design

### 3.1 Surface (`fs/partition/src/lib.rs` + `mbr.rs`, `rule.rs`)

```
pub const SECTOR_BYTES: usize = 512;
pub const TABLE_OFFSET: usize = 446;
pub const ENTRY_BYTES: usize = 16;
pub const ENTRY_COUNT: usize = 4;
pub const SIGNATURE: [u8; 2] = [0x55, 0xAA];

pub type Slot = u8;                                   // 0..=3, the MBR entry index

pub struct Entry { pub boot: u8, pub type_id: u8,
                   pub start_lba: u32, pub sector_count: u32 }   // CHS ignored

pub enum TableState { NoSignature, Empty, Valid, Corrupt }

pub struct Table { pub state: TableState,
                   pub entries: [Option<Entry>; ENTRY_COUNT] }   // None = empty slot;
                                                                 // present even when Corrupt,
                                                                 // so callers can report what they saw
pub struct Slice { pub slot: Slot, pub type_id: u8,
                   pub start_lba: u32, pub sector_count: u32 }

pub enum Exposure { Whole { table: TableState }, Partitions(Vec<Slice>) }
```

- `pub fn parse_mbr(sector: &[u8; SECTOR_BYTES], disk_sectors: u64) -> Table` — total;
  never panics; `disk_sectors` bounds the validity check.
- `pub fn is_supported(type_id: u8) -> bool` — container types `0x05`, `0x0F`, `0x85`
  and the GPT protective `0xEE` are *recognized, reported in `Table`, unsupported for
  exposure*. Everything non-zero is treated as a data partition candidate.
- `pub fn exposure(table: &Table) -> Exposure` — the curation rule (§3.2), allocation
  here is the only reason `alloc` enters the crate; acceptable (Plan 3 calls it at
  register time, not on IO paths).

### 3.2 Validity rules and the rule proper

`parse_mbr` decides `TableState` in order:

1. bytes 510/511 ≠ 55AA → `NoSignature`, all slots `None`.
2. Signature present, all 64 entry bytes zero → `Empty`.
3. Signature present. Entry consistency, checked across **all in-use entries**
   (supported or not): `sector_count > 0`; `start_lba + sector_count ≤ disk_sectors`
   (overflow-safe); no two in-use entries overlap; no in-use entry starts before LBA 1
   (nothing may contain the table itself). Any violation → `Corrupt`, with parsed
   entries still reported.
4. Otherwise → `Valid`.

`exposure(table)`:

- `NoSignature | Empty | Corrupt` → `Exposure::Whole { table }` — expose-on-corrupt,
  state carried honestly (decision 2; Plan 3 renders it as `clean`/`corrupt`).
- `Valid` with ≥1 in-use supported entry → `Exposure::Partitions(slices)` — one `Slice`
  per supported in-use entry at its slot index.
- `Valid` with in-use entries but **none supported** (extended-only disk — the honest
  edge) → `Exposure::Whole { table: Valid }` with the crate documenting why: v1 cannot
  address the container's contents, hiding the disk would brick it with no rescue tool.
  Plan 3 logs the refused container types verbatim.

Both functions are pure and total: this is the whole product. Everything above the
crate (names, kind/state wire bytes) is Plan 3's translation of these verdicts.

### 3.3 Module layout

- `mbr.rs` — constants, `Entry`, `Table`, `parse_mbr` (+ `#[cfg(test)] mod tests`).
- `rule.rs` — `Slice`, `Exposure`, `is_supported`, `exposure` (+ tests).
- `lib.rs` — re-exports; crate docs state the vocabulary (controller/disk/partition,
  volume reserved for rxfs).

## 4. Test plan (all host, `cargo test -p rxpart`)

- **Golden sectors** (hand-built `[u8; 512]` arrays): canonical single `0x83` partition
  (LBA 2048, aligned); the 63-sector legacy layout; four in-use entries; holes (only
  slots 0 and 3 used — `Slice.slot` must carry 0 and 3); boot-indicator present
  (0x80) ignored; garbage CHS bytes ignored.
- **Boundary matrix:** `end == disk_sectors` (valid), `end > disk_sectors` (Corrupt);
  `sector_count == 0` with a type set (Corrupt); `start_lba == 0` entry (Corrupt —
  contains the table); `start_lba = 0xFFFF_FFFF` overflow guard (Corrupt); tiny disk
  (`disk_sectors` < any entry end).
- **Table-state matrix:** NoSignature; all-zero table (Empty); valid; each Corrupt
  trigger in isolation.
- **Container handling:** disk with one `0x05` entry → `Valid`, `exposure` →
  `Whole { Valid }`; mixed `0x0F` + `0x83` → `Partitions` with only the `0x83` slice,
  `slot` true to the table.
- **Overlap:** two entries covering adjacent-but-not-overlapping ranges (Valid);
  overlapping ranges (Corrupt) — including the protective-MBR shape (`0xEE` spanning
  the disk alone → `Whole { Valid }` via the unsupported rule, **not** Corrupt).
- **No-panic sweep:** 10 000 seeded xorshift-generated 512-byte sectors × a few
  `disk_sectors` values — `parse_mbr` + `exposure` always return, never panic, and
  `Exposure::Partitions` slices are pairwise non-overlapping for every `Valid` table
  (a cheap invariants re-check, no deps).
- **Cross-check fixture:** the harness-built MBR bytes (the shapes `xtask`/
  `tests-integration/src/qemu.rs` stamp) parse to the expected slots/offsets.

## 5. Milestones (tiny steps; each verified before the next)

- **M0** — crate skeleton: `fs/partition` member + `default-members`, `Cargo.toml`
  (package `rxpart`, no deps, inherits workspace lints), empty `lib.rs`. *Gate:*
  `cargo test -p rxpart` (no tests yet), `cargo xtask test`.
- **M1** — `mbr.rs` constants + `Entry`/`Table`/`TableState` + `parse_mbr`, tests
  (golden sectors + boundary matrix). *Gate:* `cargo test -p rxpart`.
- **M2** — `rule.rs`: `is_supported`, `Slice`, `Exposure`, `exposure`, tests (container,
  overlap, state matrix). *Gate:* same.
- **M3** — no-panic sweep + cross-check fixture + crate docs (vocabulary, reserved
  words, the extended-only decision). *Gate:* `cargo test -p rxpart`; full
  `cargo xtask test` stays green; zero warnings.

## 6. Risks

- **Validity strictness** — refusing tables DOS-era tooling considers legal (e.g. benign
  overlaps) would expose-whole them as `Corrupt`… which is exactly the safe direction
  (operator sees the honest state, data stays addressable). The strictness bias is
  deliberate: a wrong exposure writes filesystems onto each other.
- **`exposure` allocates** (`Vec<Slice>`) — fine at register time; the rule stays out of
  IO paths (Plan 3's table is built once per register).
- Package-name drift: if the owner prefers `partition` over `rxpart`, M0 is the only
  file affected — decide at M0 review.

## 7. Evidence log convention

Same as Plan 1: Status line + trailing `## Evidence` section updated as milestones land.

---

## Evidence

Implemented 2026-11-08 on branch `blockd-partition-core` (forked from the plans-PR head), four
gated commits. Host: Linux, QEMU 10.2.1, pinned nightly toolchain; `cargo clippy -p rxpart
--all-targets` silent at every gate (0 diagnostics).

**M0** — crate skeleton (`72e8852`): workspace member `fs/partition` + `default-members` (both
lists, between `fs/fat` and `fs/ramfs`), package `rxpart`, zero dependencies, inherits workspace
lints, `lib.rs` = `#![cfg_attr(not(test), no_std)]` only. *Gate:* `cargo test -p rxpart` green
(0 tests); `cargo xtask test --skip-integration` green across the whole workspace, zero warnings.

**M1** — `mbr.rs` (`cbf2348`): constants, `Slot`, `Entry`, `TableState`, `Table`, `parse_mbr`
(total, allocation-free). In-use rule: type byte ≠ 0. Validity in order: signature 55AA, then
all-zero table → `Empty`, then `sector_count > 0 ∧ start_lba ≥ 1 ∧ start+count ≤ disk_sectors`
(u64 arithmetic, cannot wrap) and pairwise half-open-range disjointness. Tests 1–12: the golden
sectors (canonical 0x83@2048, legacy-63, four slots, holes keep indices 0 and 3, boot indicator
and garbage CHS ignored) + boundary matrix (end == disk valid; past-end, zero count, LBA 0,
u32::MAX-on-small-disk corrupt-and-still-reported; u32::MAX-on-u64::MAX disk valid — the wrap
guard proven from both sides; tiny disk corrupt). Fixture correction mid-M1: legacy-63's count
(1_985 on a 2_048-sector disk) — the parser caught its own test, production code unchanged.

**M2** — `rule.rs` (`e6d1e0d`): `is_supported` (0x05/0x0F/0x85 containers and 0xEE protective
recognized-but-refused; every other non-zero type a candidate), `Slice`, `Exposure`, `exposure`
(only allocation site in the crate, once per verdict). Tests 13–27: support matrix, every
non-Valid state → `Whole` carrying its state, extended-only → `Whole { Valid }`, mixed
0x0F+0x83 → only the 0x83 slice at its true slot, protective-MBR-spanning-the-disk → `Valid`
(not Corrupt) → `Whole { Valid }`, slices carry true slot indices across holes; in `mbr.rs`:
NoSignature (absent and one-wrong-byte), Empty, dirty type-zero bytes stay uninformative
(Valid/Empty distinction honored per §3.2), adjacent valid, one shared sector corrupt,
fully-contained corrupt.

**M3** (this commit) — no-panic sweep: 10_000 seeded xorshift rounds × 6 disk sizes (0, 1, 63,
2048, 204_800, u64::MAX); odd rounds carry the 55AA stamp so both the NoSignature and the
in-parse paths are exercised; every `Partitions` verdict re-checked (state is Valid, ≤ 4 slices,
each slice matches its true table entry and is supported, pairwise non-overlap); the sweep
asserts non-vacuity (both verdict families observed). Cross-check fixture: the
`xtask`/`tests-integration` harness shape (zeroed 1 MiB disk, bytes 510/511 = 55AA) → `Empty` →
`Whole { Empty }` — the current run-disk really is addressed whole by the rule, today's behavior
proven unchanged before Plan 3 integrates. Crate docs: vocabulary (controller → disk → partition;
*volume* reserved for rxfs), the curation rule and the expose-on-corrupt and extended-only
decisions. *Gate:* `cargo test -p rxpart` 29 tests green; full `cargo xtask test` — see below.

Full suite (`cargo xtask test`, integration included): green — every unit target and the whole
QEMU integration suite passed, all doc-test groups ok (rxpart included in the workspace run), zero
failures and zero warnings across the workspace.

## 8. Result notes

- `exposure` allocates exactly one `Vec` per decision, at register time only (Plan 3's usage);
  the parse path is allocation-free as designed.
- Decisions taken where the plan left room: none needed — the §3.2 order and the §0 rule
  transferred 1:1; the only judgment call (type-byte-only emptiness for in-use slots) was pinned
  by test `type_zero_slots_stay_empty_however_dirty_their_bytes`.
