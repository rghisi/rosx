# Plan 1 — `blockd`: the Block Device Manager, and the Refactor

> **Status:** Plan (no code written). Written 2026-11-08; decisions owner-approved in the
> design discussion of 2026-11-08.
> **Feature:** a new user-space service `blockd` ("BLOCKD") that becomes the single
> provider of record of block devices for all filesystem tools. `blockd` registers
> low-level providers (`ATA` first), presents every disk under a canonical name, and
> serves name-keyed block IO. `vfsd`, `format`, `mount` and `attach` are refactored to
> speak only `blockd` — the VFS alias registry dies. **No partition logic yet** — that is
> Plan 2 (`fs/partition`) and Plan 3 (integration); this plan lays the wire shapes so
> Plan 3 lands inside `blockd` alone.
> **Method:** every repo fact below was re-verified against the working tree at `78f65ed`.
> Facts this plan could not execute are marked **`unverified`**.

---

## 0. Decisions (owner-approved 2026-11-08)

1. **Vocabulary:** `ata0` = controller (provenance prefix only, never addressable),
   `ata0d0`/`ata0d1` = drives on the controller, partitions (Plan 3) = `ata0d0p0`…
   zero-based, slot = MBR entry index. The word **volume stays reserved for rxfs**.
2. **N2 — names are the identity** on `blockd`'s north face (name-keyed frames on a
   dedicated `"BLOCKD"` binding). IPC overhead of the hop is accepted; optimizations are
   deferred and belong only in the IPC layer later.
3. **Names are opaque to clients**: consumers obtain them from `List` and never parse the
   grammar. The grammar itself is a `blockd` convention.
4. **The alias layer dies:** VFS `ATTACH` (13) and `LIST_DEVICES` (14) retire (numbers
   stay unused, precedent: old opcode 6). `format`/`mount` address blockd device names
   directly. `attach` the app is repurposed to register a provider with `blockd`.
5. **`Register` doubles as `Rescan`**: re-issuing refreshes the namespace; no `Rescan`
   opcode exists in v1.
6. **Shape stability:** the north `Probe` reply and `List` entries carry `kind` and
   `state` bytes, and `blockd`'s internal device table carries `base`/`len`, from day one
   — in this plan always `Disk`/`Clean`/`base=0`/`len=whole`. This is what makes Plan 3 a
   `blockd`-internal change.
7. **Kernel, port grants, rxfs, `fs-core`, `RemoteBlock` ratio logic, usrlib, the vendor
   `ata` CLI: untouched.** `atad`'s numeric block face stays byte-identical.
8. `format` stays a `vfsd` opcode (standalone formatter deferred).
9. `blockd` **does not cache**. rxfs owns the cache; `blockd` is a stateless-over-table
   bridge.
10. Speak-to-`blockd` is a **norm, not an enforced boundary** (no dial-ACL in IPC); the
    direct `"ATA"` path is diagnostic-only, as recorded in AGENTS.md.

## 1. Goal & Non-Goals

### Goal
- `apps/blockd` binds `"BLOCKD"`; `attach ATA` (typed at the shell) makes `blockd` dial
  the provider, discover drives (probe until `NotFound`), and expose them as `ata0d0`,
  `ata0d1`, … with honest geometry from the provider's `Probe`.
- Name-keyed north contract in `block-protocol` (`manager` module): `Register`, `List`,
  `Probe`, `Read`, `Write`, `Flush` — payloads ride the IPC data plane exactly as the
  numeric face does today.
- `vfsd` reaches every block device through `blockd` only: `format ata0d1`, `mount ata0d1 /hdd`.
- The full `vfs_hdd_flow` persists on both arches with the new verbs.
- Zero warnings on every build surface; `cargo xtask check-isa x86_32` stays green.

### Non-Goals (each a named future step)
- **Partitions / curation** — Plan 2 + Plan 3. This plan exposes whole drives only.
- **Extended/logical, GPT, LBA48** — Plan 3 horizon / driver follow-up.
- **`fdisk`-lite authoring tool** (a vendor-`ATA`-face client) — separate plan after Plan 3.
- **Data-plane optimizations** of the hop (zero-copy relay, slice-capable delegation,
  in-process backends) — IPC-layer work with this plan's measurements as the baseline.
- **Mount-list opcode, dial ACL, secondary IDE channel, `blockd`-side caching.**

## 2. Context Map (verified at `78f65ed`)

- Numeric provider face (stays untouched): `protocol/block/src/wire.rs` — `PROBE=0x20`,
  `READ=0x21`, `WRITE=0x22`, `FLUSH=0x23`, replies `0x30–0x33`, `FRAME_BYTES=60`,
  `MAX_BLOCKS_PER_REQUEST=128`, `DESCRIPTION_BYTES=47`, `BlockError{NotFound=1,
  BadAddress=2, Busy=3, Device=4, Unsupported=5, Protocol=255}`; typed client
  `protocol/block/src/client.rs` — `Block::connect(service)` (50×100 ms retry),
  `probe/read_blocks/write_blocks/flush/disconnect`, `Geometry`.
- `atad` serves the vendor face + numeric block face on `"ATA"`
  (`apps/atad/src/main.rs:107,134-137`); drives 0/1 (`probe_drives`, `:69-77`);
  `block_drive_of` maps numeric device → drive (`:316-323`).
- `vfsd` today (`apps/vfsd/src/fs.rs`): `AttachedDevice` `:28-35`, `devices` vec and
  `attach_device` `:146-176`, `attached` `:178-183`, `open_remote_block(service, device)`
  `:185-200`, `format_device` `:202-218`, `mount_device` `:220-243`, `MountedVolume`
  `:37-41`, `devices_blob` `:308-327`. VFS dispatch in `apps/vfsd/src/main.rs:120-146`.
- VFS wire (`protocol/vfs/src/wire.rs`): `MOUNT=9`, `FORMAT=10`, `ATTACH=13`,
  `LIST_DEVICES=14`; `Request::Mount { device, dir }` budget `MOUNT_STRING_BUDGET=57`;
  `Request::Attach` budget `ATTACH_STRING_BUDGET=53`; client
  `protocol/vfs/src/client.rs` `mount/format/attach/list_devices`.
- `RemoteBlock` (`apps/vfsd/src/blk.rs`): ratio adapter `device_block = block * ratio`,
  ratio from `BLOCK_BYTES % device_block_size`; implements `rxfs::device::BlockDevice`.
- CLI apps: `apps/attach/src/main.rs` (51 lines: `attach <id> <service> <device>` /
  `attach list`, dials `Vfs`); `apps/format`, `apps/mount` dial `Vfs`.
- Workspace: members are explicit (`Cargo.toml:2-51`); `apps/*` and `protocol/*`
  patterns: `protocol/*` is a glob (new module needs no member edit), `apps/blockd`
  must be added to `members`. `USER_APPS: [&str; 30]` in `xtask/src/main.rs:11` grows
  to 31 (both arches, ROM packing at `:94`, run-disk at `:249` follow automatically).
- Boot plans: `arch/x86_64/src/main.rs:84` and the x86_32 twin list
  `.service("atad", ata_protocol::wire::SERVICE_NAME)`; doc example
  `system/src/boot_plan.rs:163` `run("attach", &["hdd", "ATA", "0"])` must become
  `run("attach", &["ATA"])`.
- Integration: `tests-integration/tests/vfs_hdd_flow.rs` (the flow this plan re-speaks);
  harness `tests-integration/src/qemu.rs` `spawn_with_hdd_disk` `:43`, scratch/raw
  images `:64-85`. Vendor tests `ata_identify/ata_read/ata_write` are untouched.

## 3. Design

### 3.1 North contract — `block-protocol::manager`

New module `protocol/block/src/manager/{wire.rs, client.rs}`; `lib.rs` gains
`pub mod manager`. Reuses `BlockError` from the existing `wire` module unchanged.
A dedicated binding means a fresh opcode space is safe (the 0x20–0x2F band discipline
exists only because two faces share `"ATA"`).

```
SERVICE_NAME   = "BLOCKD"        FRAME_BYTES = 60      NAME_BYTES = 24
REGISTER = 0x01   [1]=service_len [2..2+len] service                 budget 58
LIST     = 0x02   (bare)
PROBE    = 0x03   [1]=name_len [2..26] name
READ     = 0x04   [1]=name_len [2..26] name [26..34] block u64 [34..36] count u16 (1..=128)
WRITE    = 0x05   READ frame + [36..40] data-token u32
FLUSH    = 0x06   PROBE frame shape
REPLY_ACK = 0x10 | REPLY_ERROR = 0x11 (+ BlockError byte) | REPLY_DATA = 0x13 (+ token u32)
REPLY_PROBE = 0x12 [1..9] block_count u64 | [9..13] block_size u32 (power of two)
           [13] kind u8 | [14] state u8 | [15..59] description (44 B, printable-normalized, space-padded)
```

- `kind`: `DISK = 0`, `PARTITION = 1` (Plan-3 value; codec accepts both now).
- `state`: `CLEAN = 0`, `PARTITIONED = 1`, `CORRUPT = 2` (Plan-3 values; codec accepts all now).
- `List` replies `REPLY_DATA`; the data object is `count u32` (LE) + fixed **112-byte**
  entries: `name[24] source[24] description[44] block_count u64 block_size u32
  kind u8 state u8 spare[6]`.
- Name validity (codec-enforced): length `1..=24`, bytes from `[a-z0-9-]`, no leading `-`.
- Typed client `manager::client::Manager`: `connect()` (dials `"BLOCKD"`, retry like
  `Block::connect`), `register(service) -> Result<(), BlockError>`,
  `list() -> Result<Vec<DeviceInfo>, …>`, `probe(name)`, `read_blocks(name, block,
  blocks)`, `write_blocks(name, block, data)`, `flush(name)`, `disconnect`.
  `DeviceInfo` mirrors the entry fields as owned types.

### 3.2 `blockd` internals (`apps/blockd`)

Shape mirrors `atad` (lib target for host-testable core + thin `main.rs`; `#[cfg(test)]`
modules only).

- `Provider` trait (internal): `probe(drive) / read / write / flush` over the numeric
  face. Production impl `AtaProvider` holds the live `block_protocol::client::Block`
  dialed to `"ATA"`; host tests get a scripted fake (the trait is production-used, so the
  no-test-only-surface rule holds).
- `DeviceEntry` (the table): `name, source, description, kind, state, provider slot,
  provider_device: u32, base: u64, len: u64`. In this plan: `kind=Disk`, `state=Clean`,
  `base=0`, `len=whole-device count` — all forwarding arithmetic (`device_block =
  base + block`, range-check against `base+len`) is real code now, Plan 3 only changes
  how entries are *built*.
- `Register { service }`: `"ATA"` maps to `AtaProvider`; any other string →
  `Unsupported` (honest: provider kinds are compiled in). Re-`Register` of a live
  service tears down its entries and re-probes (refresh). Discovery: numeric probe
  `device = 0,1,2,…` until `NotFound`, capped at 8.
- Names: `ata{controller}d{drive}`, controller = `0` in v1 (primary channel only).
- `Probe/Read/Write/Flush { name }`: table lookup (`NotFound` on miss) → forward with
  the `base`-offset translation; `count` capped at 128 like the numeric face.
- Data plane: drain the provider's `REPLY_DATA` token in 4 KiB chunks, re-publish a
  `UserDataHandle` per owner exactly like `atad`'s held-token pattern
  (`apps/atad/src/main.rs:243-258`); release on the owner's next request, same discipline.
- One persistent provider dial per registered service; provider faults surface as
  `Device`/`Busy` honestly; no reconnect cleverness in v1.
- `blockd` starts with an empty namespace (no providers auto-discovered).

### 3.3 `vfsd` refactor

- Delete: `AttachedDevice`, `devices`, `attach_device`, `attached`, `devices_blob`, the
  `ATTACH`/`LIST_DEVICES` dispatch arms (`main.rs:134-146`), client `attach`/
  `list_devices`, `Request::Attach`/`ListDevices` (opcodes 13/14 stay unused).
- `open_remote_block(name)` now: `manager::Manager::connect()` → `probe(name)` →
  `RemoteBlock::new(...)`. `apps/vfsd/src/blk.rs` swaps its client type from
  `client::Block` to `manager::client::Manager` and addresses by name; the ratio math,
  `BlockDevice` impl and error mapping are otherwise line-for-line the same.
- `format_device(name)` / `mount_device(name, dir)`: take the blockd device name; busy
  guard = "this name is in the mount table"; mount-point discipline (`mkdir` first,
  overlay, `rm`/`rmdir` refusal), geometry veto (`Unsupported`) and `sync()`-after-write
  are untouched. `MountedVolume.device` now stores the blockd name.
- `error_map::block` is reused for manager errors unchanged (`NotFound` on unknown name
  before any dial — a `mount ata0d1 /hdd` before `attach ATA` fails `NotFound`).

### 3.4 `attach` app repurposed

- bare `attach` → `Manager::list()`, render the tree grouped by `source`
  (`ata0` controller header, drives, geometry, state).
- `attach <service>` → `Manager::register(service)`; prints discovered devices (the
  post-register `list()`).
- Unknown verbs → usage line; no `Vfs` dependency left in the app.

### 3.5 Flow (post-refactor happy path)

```
boot (plan: … atad, blockd, …)   →  attach ATA
rosx> attach            →  ata0: ata0d1  QEMU HARDDISK 102400x512  clean
rosx> format ata0d1     →  rosx> mkdir /hdd     →  rosx> mount ata0d1 /hdd
```

## 4. Milestones (tiny steps; each verified before the next)

- **M0** — `manager::wire` codec + full byte-level unit tests (round-trips, zero-tail
  rejection, name validation, kind/state accept, budget bounds). *Gate:*
  `cargo test -p block-protocol`.
- **M1** — `manager::client` typed client. *Gate:* same; host-side (no consumer yet).
- **M2** — `apps/blockd` skeleton: bind, serve loop shape, `Register` with the
  `AtaProvider` dial + probe-scan, device table with `base/len` arithmetic, north
  `Probe/List/Read/Write/Flush`. `Provider` trait + fake in `#[cfg(test)]`; unit tests
  for register/refresh, unknown service `Unsupported`, name lookup, `base/len` bounds,
  error mapping. Workspace member + `USER_APPS` += `blockd`. *Gates:* `cargo test -p
  blockd`, `cargo xtask build` (x86_64), then `cargo xtask build x86_32` +
  `cargo xtask check-isa x86_32`.
- **M3** — boot plans (`arch/x86_64/src/main.rs`, `arch/x86_32/src/main.rs`): insert
  `.service("blockd", block_protocol::manager::SERVICE_NAME)` after `atad`; fix the doc
  example + test expectation in `system/src/boot_plan.rs:163-165`. *Gate:*
  `cargo test -p system`; `cargo run` reaches shell with `[BLOCKD] ready` in the log.
- **M4** — `attach` app repurposed (§3.4). *Gate:* manual `cargo run`: `attach ATA`
  lists `ata0d1` (QEMU single scratch disk on slave slot; master is the boot image).
- **M5** — VFS wire + client deprecations (§3.3), `blk.rs` client swap. *Gate:*
  `cargo test -p vfs-protocol`, kernel-free.
- **M6** — `vfsd` refactor (§3.3). *Gates:* `cargo test -p vfsd` (existing unit tests
  reshape with the registry), `cargo xtask build`, integration:
  `cargo test -p tests-integration -- vfs --test-threads=1` with the rewritten
  `vfs_hdd_flow` (M7 lands the rewrite; M6 keeps at least `ipc_vfs` green).
- **M7** — integration rewrite: `vfs_hdd_flow` to the new verbs — `attach ATA`;
  `mount ata0d1 /hdd` **before** register → `NotFound`; register-twice is now a refresh
  (assert success, not `Busy`); `format ata0d1` while mounted → `Busy`; persistence on
  second boot unchanged. *Gate:* full suite, both arches
  (`ROSX_ARCH=x86_32 cargo test -p tests-integration -- --test-threads=1`).
- **M8** — measurement + docs: record the `vfs_hdd_flow` wall time before/after the hop
  into the evidence log; AGENTS.md "Disk architecture" and "VFS architecture" paragraphs
  rewritten (blockd namespace + vocabulary + norm-not-enforcement + new persistent-disk
  workflow `attach ATA` → `format ata0d1` → `mount ata0d1 /hdd`); this file's Status
  updated. *Gate:* `cargo xtask test`; full review pass.

## 5. Verification

- Unit: `block-protocol` (M0/M1), `blockd` (M2), `system` (M3), `vfs-protocol` (M5),
  `vfsd` (M6).
- Integration both arches: rewritten `vfs_hdd_flow`; `ata_*` vendor tests must stay
  green untouched; `ipc_vfs`, `kstats_mem` unaffected.
- ISA: `check-isa x86_32` green (pure integer code).
- Zero warnings on all build surfaces (workspace lint policy).

## 6. Risks

- **Failure chain gains a link** (vfsd→blockd→atad): either dying surfaces as
  `IoError::Device` mid-mount. Accepted; documented in AGENTS.md; reconnect-per-request
  explicitly rejected for v1 (it changes release-token semantics).
- **The hop's cost** is unquantified today — M8 fixes that with a number, which is the
  precondition for the deferred IPC optimizations.
- **Frame/name budget:** `NAME_BYTES=24` covers `ata0d1p3` and headroom for GUID-ish
  Plan-3/GPT names; raising it later touches the wire, so 24 is chosen generously now.
- **ROM image grows** by one small ELF (USER_APPS → ROM packing at `xtask:94`).

## 7. Evidence log convention

This file's Status line and a trailing `## Evidence` section track completion (commands,
wall times, notable output), matching the `plans/*-impl.md` house pattern in-place.
