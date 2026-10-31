# ATA (IDE) PIO Disk Driver in User Space — Implementation Plan

> **Status:** Plan (no code written). Written 2026-10-30; decisions recorded 2026-10-31.
> **Feature:** `atad` — a user-space ATA (IDE) PIO-mode disk daemon exposing its external API
> over IPC, plus the kernel-mediated port-I/O capability it needs, the `ata-protocol` wire
> treaty crate, and an `ata` demo client.
> **Method:** every repo fact below was re-verified against this working tree (paths + lines
> cited). Commands this plan cannot prove green are marked **`unverified`** and routed to the
> orchestrator's baseline pass before they become binding gates.
>
> **Decisions (owner-approved 2026-10-31):**
> - **Greenfield, legacy archived.** The unmerged `legacy-ide-pio` branch (branched at 684bd73:
>   packed `PortOp`/`IrqCtl` syscalls, one-owner grant registry, IRQ14 event-driven reads, an
>   `apps/atad` core, but no external IPC API and read-only, and five commits behind on
>   pre-handle-rework history) is kept for reference only. This plan proceeds independently;
>   the branch's IRQ14 manual-ack and dispatch-core `configure` ideas are mined when M7
>   delivers event completion.
> - **Polling v1, IRQ14 as v2.** Bounded spin + `sleep(1)` fallback per §5.4 stands; the
>   `deliver_to_service` event path (PS/2 precedent) is the named v2 upgrade, not v1 scope.

---

## 1. Goal & Non-Goals

### Goal
- v1 delivers, on **x86_64 only**: primary IDE channel, compat I/O addressing
  (task file `0x1F0–0x1F7`, alternate status `0x3F6`), **both drives selectable**
  (master = 0, slave = 1 — the slave slot is what gives the write-path test a scratch
  disk without touching the boot disk), IDENTIFY (presence, model, firmware, LBA28
  capacity), READ SECTORS (LBA28, 1–128 sectors/request), WRITE SECTORS, FLUSH CACHE.
- The driver lives **entirely in user space** as `atad` (apps/atad), modeled on `vfsd`/`ps2d`.
- The external API is a **wire treaty** in a new `ata-protocol` crate (modeled on
  `vfs-protocol`/`input-protocol`), with bulk sector data on the **IPC data plane**
  (`IpcDataWrite`/`IpcDataRead`/`IpcDataRelease` tokens — a 512-byte sector never fits a
  60-byte message payload).
- The kernel stays the **single authority on invariants**: user space gets no raw port
  access; it gets kernel-mediated, grant-checked port syscalls behind a HAL trait, so the
  x86 port concept never appears as *code* inside `kernel/`.
- Polling (bounded spin + sleep fallback) for DRQ/BSY/ERR is accepted for v1 (rationale in
  §5.4); IRQ14 event delivery is the named v2 upgrade path because the PS/2 precedent makes
  it cheap (§9).

### Non-Goals (explicit, each a named future step in M7)
- **VFS/vfsd integration**: the ATA disk is NOT mounted into the VFS; `vfsd` stays a RAM/ROM
  filesystem. A block-device backend for `vfsd` sits on top of `ata-protocol` later.
- **DMA** (PDMA/UDMA): non-goal. PIO only.
- **LBA48**: deferred to v2. Justification: LBA28 covers 128 GiB — far beyond any hobby QEMU
  image; the LBA28 task-file sequence is single-shot (no count-0-means-65536 rule, no
  register-pair writes). Upgrade path: gate on IDENTIFY word 83 bit 10, add LBA48 commands
  `0x24`/`0x34` with the two-phase register writes.
- **PCI enumeration / BAR discovery**: v1 hard-codes the compat base `0x1F0/0x3F6` as the
  policy grant at boot. QEMU (piix3 *and* q35) exposes IDE at the compat bases; real hardware
  typically matches, and the milestone-0 spike proves it for the test environment. Future:
  PCI scan, use BARs when they differ from compat.
- **Secondary IDE channel** (`0x170–0x177`/`0x376`), **SET MULTIPLE**, **x86_32 enablement**,
  **real hardware**: future. The port-I/O mechanism itself is x86-common and `atad` /
  `ata-protocol` are arch-neutral so x86_32 can adopt them later (its KConfig just wires no
  grants in v1, so the syscalls safely return `PortAccessDenied` there).

---

## 2. Verified Baseline Facts (re-checked in this tree, 2026-10-30)

These are the load-bearing facts. Citations are `path:line`.

1. **No port-I/O syscall exists.** `system/src/syscall_numbers.rs:3-23` ends at
   `RomRead = 27`; numbers 4 and 9 are pinned retired by
   `read_char_and_try_read_char_numbers_stay_retired` (`system/src/syscall_numbers.rs:59-62`);
   gaps 13,14,16–19,21 are unreissued. The live-number test
   `every_live_number_round_trips_through_try_from` (`:65-90`) enumerates every live variant —
   **any new number must be added to that array** (blast radius).
2. **Message treaty:** `Message` = 64 B = `conn: u32` + `[u8; 60]` payload
   (`system/src/ipc.rs:1-26`); `KERNEL_EVENT_CONN = 0x8000_0000` (`:3`).
3. **Data plane caps:** `MAX_DATA_BYTES = 64 * 1024` per object and
   `DATA_BUDGET_BYTES = 256 * 1024` per *writing task*
   (`kernel/src/ipc/data_manager.rs:8-9`). ⇒ a request ≤ 128 sectors (64 KiB).
4. **HAL injection pattern:** `KConfig { cpu: &'static dyn Cpu, elf_arch: &'static dyn ElfArch,
   scheduler_factory }` (`kernel/src/kconfig.rs:5-9`); the only two construction sites are
   `arch/x86_64/src/main.rs:43-47` and `arch/x86_32/src/main.rs:17-21` (verified by grep —
   no test constructs `KConfig`).
5. **PC-specific concepts already live portably behind traits in `kernel/`:**
   `kernel/src/interrupt.rs` deals in IRQ numbers/vectors via the `InterruptController`
   trait + `install::<C>()` static wiring (`:24-70`), with host unit tests against a
   `MockController` (`:114-294`). This is the structural precedent for a portable
   port-capability module. (`kernel/src/ports/` is hexagonal driving/driven seams — NOT x86
   ports; the new module must not be confused with it.)
6. **All ELF tasks are named `"ELF"`** (`kernel/src/task.rs:243-251`) — so **task-name-based
   grant policy is impossible**; grants must key on the `TaskHandle` returned by
   `kernel.schedule(...)` (`arch/x86_64/src/main.rs:89-94` shows the embed pattern:
   `include_bytes!("../../../target/rosx-user/release/<name>")` + `new_elf_task`).
7. **Current task is observable in syscall context:** `kernel().execution_state.current_task()`
   (`kernel/src/syscall.rs:58`, `kernel/src/state.rs:70-72`).
8. **Error-code treaty:** `ErrorCode` families `0x01xx` IPC / `0x02xx` ELF
   (`system/src/error.rs:9-25`); the test array `ALL_ERROR_CODES: [ErrorCode; 15]` (`:57-73`)
   **must be grown** when variants are added. Error register convention: `ERROR_BIT = 1<<31`,
   `is_token(raw) = raw < ERROR_BIT` (`:1-4`) — **a 32-bit port read value ≥ 0x8000_0000 would
   alias an error**, hence the out-pointer read design in §3.3.
9. **Daemon precedents:** `apps/ps2d/src/main.rs:14-23` (bind-retry loop),
   `apps/vfsd/src/main.rs:73-96` (bind → `ipc_accept_message` → `wait_for_message` →
   dispatch on `received.connection()` → `ipc_send(reply)`), and the per-owner held-data-object
   list with `release_held_for(owner)` before dispatch (`:98-107`, `:183-199`).
   `deliver_to_service(service, message) -> bool` exists (`kernel/src/ipc/mod.rs:8-12`) and is
   used from arch IRQ handlers (`arch/x86_64/src/interrupts.rs:117`).
10. **Protocol crate template:** `vfs-protocol` (`Cargo.toml` deps: `system`, `usrlib` only);
    `wire.rs` pins `FRAME_BYTES = 60`, opcode bytes 1.., reply tags `0x10..`, `VfsError`
    `#[repr(u8)]` with `Protocol = 255` fallback, byte-offset pinning tests
    (`vfs-protocol/src/wire.rs:1-35, 272-746`); `client.rs` typed client with
    connect-retry and `drain_data_object` in 4096-byte chunks (`vfs-protocol/src/client.rs:12-53`).
11. **Client-app template:** `apps/mem/src/main.rs` (connect-retry to a service, send request,
    wait reply, print report). App crate template: `apps/vfsd/Cargo.toml` (lib + bin with
    `test = false` on the bin; host tests live in the lib).
12. **xtask:** `USER_APPS: [&str; 16]` (`xtask/src/main.rs:5`) drives both user-ELF builds and
    the ROM roster (`pack_rom`, `:76-99`); `cargo xtask test` runs
    `cargo test --workspace -- --test-threads=1` after building apps + ROM, with
    `--exclude tests-integration` under `--skip-integration` (`:101-114`).
13. **QEMU:** the runner launches `qemu-system-x86_64 -drive format=raw,file=<boot.img>
    -debugcon stdio -no-reboot -no-shutdown -d cpu_reset`
    (`arch/x86_64-runner/src/main.rs:19-26`) — **`-drive` without `if=` attaches to the
    machine's default interface, which on x86 PCs (piix3 and q35) is IDE**; milestone 0 proves
    this empirically. The test harness spawns QEMU directly with the same `-drive` shape plus a
    HMP monitor socket (`tests-integration/src/qemu.rs:37-53`); `Monitor::connect` and
    `Monitor::send_command` are **public** (`tests-integration/src/monitor.rs:18,40`) and return
    the monitor's textual reply (`:40-46`), so HMP port commands can be scripted today with no
    harness change.
14. **Integration test template:** `tests-integration/tests/kstats_mem.rs` (spawn → expect
    `"rose>"` → mark → `send_line("mem")` → expect report needles → expect prompt return).
    Nine test files exist; all assertions are substring/needle-based, so an *added* ROM roster
    entry or boot banner cannot break them by addition (verified by grepping their needles).
15. **CI:** `ci.yml` runs exactly `cargo xtask test --skip-integration` and
    `cargo xtask build`; `build-artifacts.yml` is workflow_dispatch running `cargo xtask build`.
    **Integration (QEMU) tests do not run in CI today.**
16. **Docs:** `CLAUDE.md` and `GEMINI.md` are `@AGENTS.md` includes — only `AGENTS.md` needs
    edits. `docs/` currently holds `todo.md` and `app-elf-size-investigation.md`.
17. **Repo style:** edition 2024, nightly, **no comments ever** (assembly is the only
    exception), `kernel/` is `no_std` depending only on `system` + `lazy_static`, host unit
    tests mandatory for kernel modules (`AGENTS.md`), commit messages carry **no prefix**.

---

## 3. The Hardware-Access Decision

### 3.1 Options evaluated

**(a) Kernel-mediated port-I/O syscalls, authorized by a per-task port-range grant, behind a
HAL trait. — CHOSEN.**
- *Kernel as single authority on invariants:* the grant check is kernel code executed on every
  access; the driver re-checks nothing. The kernel never interprets ATA semantics — it only
  enforces "this task may touch these 16-bit port numbers at this width". This is exactly the
  split the rework codified: domain knowledge (ATA sequencing) in the serving subsystem (the
  `atad` daemon), invariant enforcement (port capability) at the kernel edge.
- *HAL rule:* the `in`/`out` instructions are hardware-specific routines and live in
  `arch/x86_64` / `arch/x86_32`. The portable kernel sees only a trait + a numeric grant
  table — structurally identical to `kernel/src/interrupt.rs`, which is portable code that
  nonetheless speaks in IRQ numbers and vectors (fact 5). The precedent is in-tree, not
  idealized.
- *`system` holds numbers only:* the syscall numbers and error codes are pure numbers; the
  grant mechanism is kernel mechanism; the ATA wire contract is a protocol crate. Nothing
  mechanism-shaped enters `system`.
- *Translation at the boundaries:* the kernel edge validates + executes; user space calls
  typed `usrlib` wrappers; the daemon translates domains (sectors ↔ task file); the kernel
  translates nothing but a trap into an `in`/`out`.

**(b) IOPL elevation for the driver task. — REJECTED.**
- No granularity: `RFLAGS.IOPL=3` lets *that task* touch *every* port (including the PS/2
  data port 0x60 the kernel itself services, the timer, the DMA controller), violating
  "single authority on invariants" — the invariant would be enforced by a CPU flag, not by
  kernel code, and once raised the kernel cannot tell who touched what.
- Per-task IOPL requires rewriting saved RFLAGS on every context switch
  (`context_switching.S`, `Task::prepare_entry`), threading an x86 privilege concept through
  the *portable scheduler* — the single worst place to leak arch state.
- Not portable to any non-x86 target (the stated m68k/ARM/RISC-V future has no IOPL), and
  x86_32 Multiboot boots ring-0 anyway, so the policy would diverge per arch.
- The one thing it buys — speed — is matched by (a)'s block syscalls (§3.3) at 1 trap per
  sector instead of 256.

**(c) Kernel executes whole ATA operations. — REJECTED.** Directly contradicts the goal
(driver lives in user space) and inverts the established architecture: `ps2d`/`inputd`/`vfsd`
show device *semantics* live in user-space daemons; the kernel only ships opaque frames. A
kernel ATA engine would also bloat the portable `kernel/` with x86 domain code.

### 3.2 Chosen design — overview

New portable module **`kernel/src/portio.rs`** (deliberately *not* under `ports/`, which is
the hexagonal seam module): a `PortIo` HAL trait (wired through `KConfig` exactly like `Cpu`
and `ElfArch`), a static grant table keyed by `TaskHandle`, and the authorization predicate.
New x86 impls `arch/x86_64/src/portio.rs` and `arch/x86_32/src/portio.rs` hold the `in`/`out`
assembly. **Boot policy lives at the arch boundary**: `arch/x86_64/src/main.rs`, after
scheduling the `atad` ELF, grants that task `0x1F0–0x1F7` and `0x3F6`. The kernel knows *what
a grant is*; the arch boot sequence decides *who has one*. x86_32 wires the trait but grants
nobody (deny-by-default; its syscalls return `PortAccessDenied`).

Why boot-time `TaskHandle` grants and not "whoever binds the `ATA` service gets the ports":
binding races let any early binder win raw disk access, capability would hinge on scheduling
luck, and authorization by runtime accident is precisely the inversion AGENTS.md warns about.
Why not task *name*: every ELF task is `"ELF"` (fact 6) — names cannot discriminate.

### 3.3 Syscall treaty sketch (prose only)

Numbers continue the existing treaty monotonically (retired 4/9 stay retired; legacy gaps are
not reissued):

1. **`PortIn = 28`** — read one value from one port. arg1 = port (u16), arg2 = width in
   bytes (1, 2, or 4), arg3 = destination pointer to a `u32` slot the kernel fills
   (zero-extended). Returns 0 on success or an error-code register. **The out-pointer is
   mandatory**, because a 32-bit value can be ≥ `ERROR_BIT` and would alias the error
   convention (fact 8); this mirrors the existing `WaitFuture` out-pointer idiom
   (`kernel/src/syscall.rs:39-51`). Do not "simplify" to a direct return value.
2. **`PortOut = 29`** — write one value. arg1 = port (u16), arg2 = value (only the low
   `width` bytes used), arg3 = width (1, 2, or 4). Returns 0 or error.
3. **`PortInBlock = 30`** — repeated 16-bit input: read `count` words from the same port
   into a caller buffer (the data-port fast path: one trap per sector, not 256). arg1 =
   port, arg2 = buffer pointer, arg3 = word count (valid range 1..=32768; 0 or larger
   rejected as invalid). Returns 0 or error.
4. **`PortOutBlock = 31`** — same, 16-bit repeated output. arg1 = port, arg2 = buffer
   pointer, arg3 = word count (same bound). Returns 0 or error.

Per-syscall kernel validation, in order (each failure returns the error register, never a
partial I/O):
- width ∈ {1, 2, 4} (block ops imply width 2) — else `PortRequestInvalid` (new, §3.5).
- `port + width ≤ 0x10000` (no wraparound) — else `PortRequestInvalid`.
- block `count` ∈ 1..=32768 (caps a user request to 64 KiB, matching the data-plane cap) —
  else `PortRequestInvalid`.
- the **current task** (from `ExecutionState`) holds a grant covering the touched span —
  else `PortAccessDenied` (new, §3.5). For single accesses the span is `[port, port+width)`;
  for block ops the access repeats one port, so the grant must merely cover `port..port+2`.

Authorization semantics: grants are inclusive `[base, end]` ranges keyed by the full
`TaskHandle` **including its generation** — a recycled slot yields a different generation, so
a stale grant never matches a new task. Table capacity 8 entries; `grant()` beyond capacity
fails and the arch boot wiring `kprintln!`s the failure (visible on the debug console).

### 3.4 HAL trait boundary (who defines, who implements)

- **Defined (portable):** `kernel/src/portio.rs` — trait `PortIo` with four behaviors, in
  prose: read a 1/2/4-byte value from a port (pre-validated); write a 1/2/4-byte value to a
  port (pre-validated); read `n` u16 words from one port into a slice; write a slice of u16
  words to one port. The trait is added to `KConfig` as
  `pub port_io: &'static dyn PortIo` (mirroring `cpu`/`elf_arch`, fact 4); `Kernel::new`
  stores it like `elf_arch`. Both `KConfig` literals (both arch `main.rs`) gain the field in
  the same step — no intermediate step may leave either arch unbuildable.
- **Implemented (arch):** `arch/x86_64/src/portio.rs` and `arch/x86_32/src/portio.rs` —
  small structs implementing `PortIo` with inline `asm!` for `in`/`out` (operand constraint
  admitting immediate-or-DX port addressing; the arch/x86_32 helpers `inb`/`outb` already in
  `arch/x86_32/src/interrupts.rs:140,165` show the house idiom). Block forms may be plain
  word loops (acceptable for v1); `rep insw`/`rep outsw` is an optional optimization.
  Assembly/comment rule: inline asm operand/register notes are the one permitted comment
  site; no other comments anywhere.
- **Denied world:** with no `port_io` wiring there is no valid `KConfig`, so the field is
  mandatory — the type system forces both arches to acknowledge the capability while only
  x86_64 grants anyone.
- **Interrupt/context notes:** port syscalls run in syscall (task) context with interrupts
  on; the granted range `0x1F0–0x1F7/0x3F6` is disjoint from every port the kernel itself
  touches (timer `0x40–0x43`, PIC `0x20/0xA0`, keyboard `0x60/0x64`, debug console), so
  interleaved IRQs cannot collide. On single-core there is no yield point between the grant
  check and the instruction sequence inside one handler, so no check-use race exists.

### 3.5 Error codes (numbered list continuing the treaty)

- `PortAccessDenied = 0x0301` — no grant covers the access (or the grant vanished).
- `PortRequestInvalid = 0x0302` — malformed width/count/port (caller bug).

Blast radius (same step, same file): `ErrorCode::from_code` match arms and
`ALL_ERROR_CODES: [ErrorCode; 15]` → `[ErrorCode; 17]` in `system/src/error.rs:57-73`.

### 3.6 usrlib wrappers (user-side edge)

`usrlib/src/syscall.rs` gains four wrappers mirroring the existing `Result<_, E>` idiom, and
`usrlib/src/error.rs` gains a `PortError { Denied, Invalid }` mapped from the two codes
(mirroring the `DataError` pattern):
- `port_in(port: u16, width: usize) -> Result<u32, PortError>` (stack slot out-pointer
  passed as arg3).
- `port_out(port: u16, value: usize, width: usize) -> Result<(), PortError>`.
- `port_in_words(port: u16, dst: &mut [u16]) -> Result<(), PortError>`.
- `port_out_words(port: u16, src: &[u16]) -> Result<(), PortError>`.

---

## 4. Wire Contract — `ata-protocol` (the external API)

Service name: **`"ATA"`** (`wire::SERVICE_NAME`, mirroring `vfs-protocol` which owns
`"VFS"`). Frame sizes: `FRAME_BYTES = 60` (message payload), `SECTOR_BYTES = 512`,
`MAX_SECTORS_PER_REQUEST = 128` (data-object cap 64 KiB ÷ 512, fact 3) — pinned by codec
tests so the u8 sector-count field can never smuggle 129..=255 past the data plane.

### 4.1 Tag table (request opcodes `0x01..`, reply tags `0x10..`, disjoint like VFS)

| Byte 0 | Name        | Direction  | Meaning                                  |
|--------|-------------|------------|------------------------------------------|
| 0x01   | IDENTIFY    | request    | probe drive + report identity            |
| 0x02   | READ        | request    | read sectors → data-plane object         |
| 0x03   | WRITE       | request    | write sectors from a data-plane object   |
| 0x04   | FLUSH       | request    | flush write cache                        |
| 0x10   | REPLY_ACK   | reply      | success with no payload                  |
| 0x11   | REPLY_ERROR | reply      | failure + `AtaError` + device snapshots  |
| 0x12   | REPLY_IDENT | reply      | identity + capacity                      |
| 0x13   | REPLY_DATA  | reply      | data-plane token + sector count          |

### 4.2 Request byte layouts (little-endian; all unspecified bytes pinned zero; decode
enforces the stated bounds and returns `AtaError::Protocol` on any violation)

| Request  | [0]  | [1]         | [2..6]   | [6]   | [7..11]   | Bounds enforced at decode                            |
|----------|------|-------------|----------|-------|-----------|------------------------------------------------------|
| IDENTIFY | 0x01 | drive (0/1) | —        | —     | —         | drive > 1 → reject                                   |
| READ     | 0x02 | drive (0/1) | lba u32  | count | —         | `lba & 0xF000_0000 ≠ 0` → reject (LBA28); count = 0 or count > 128 → reject |
| WRITE    | 0x03 | drive (0/1) | lba u32  | count | token u32 | same bounds; the object must hold exactly `count * 512` bytes (daemon-checked) |
| FLUSH    | 0x04 | drive (0/1) | —        | —     | —         |                                                      |

### 4.3 Reply byte layouts

| Reply       | [0]    | [1..5]     | [5..7]        | [7..60] zero tail | extra fields                                                     |
|-------------|--------|------------|---------------|-------------------|------------------------------------------------------------------|
| REPLY_ACK   | 0x10   | (all zero — zero-tail enforced on decode, per the `vfs` `zero_tail` idiom) | | | |
| REPLY_ERROR | 0x11   | [1] code byte = `AtaError`; [2] raw ATA error register (0 when not device-originated); [3] raw last status byte (0 when N/A); [4..60] zero | | | |
| REPLY_IDENT | 0x12   | —          | —             | [57..60] zero     | [1..9] sectors u64; [9..17] firmware rev 8 ASCII; [17..57] model 40 ASCII |
| REPLY_DATA  | 0x13   | token u32  | sectors u16   | [7..60] zero      |                                                                  |

IDENTIFY words 27–46 (model) and 23–26 (firmware) arrive byte-swapped per the ATA string
convention; **atad normalizes** before encoding — clients only ever see plain ASCII.
Non-printable identity bytes are space-padded/truncated by atad, never re-sent raw.

### 4.4 `AtaError` (mirrors `VfsError` style, `#[repr(u8)]`, unknown byte → `Protocol`)

| Byte | Variant      | Meaning                                                    |
|------|--------------|------------------------------------------------------------|
| 1    | NotFound     | no device at that drive (IDENTIFY signature absent)        |
| 2    | BadAddress   | daemon-side bounds: LBA beyond identified capacity, count invalid |
| 3    | Busy         | BSY never cleared within the bounded wait (timeout)        |
| 4    | Device       | ERR bit set; err/status snapshot bytes carry the device state |
| 5    | Denied       | port syscall refused (capability lost) — daemon cannot operate |
| 255  | Protocol     | malformed frame / unexpected reply shape                   |

### 4.5 Data-plane flow

- **READ:** atad transfers `count * 512` bytes into one object via `Syscall::ipc_data_write`
  and replies `REPLY_DATA { token, sectors }`. atad **holds** the object (per-owner held list
  exactly like `vfsd`'s `held` + `release_held_for`, fact 9) and releases it on the owner's
  next request. Client drains in 4096-byte chunks (like `vfs-protocol`'s
  `drain_data_object`) and the typed client releases the handle explicitly after draining.
- **WRITE:** client uploads payload via `ipc_data_write`, packs the token, sends the WRITE
  request; atad drains the object (vfsd `drain_incoming` idiom), performs the writes, replies
  ACK/ERROR; client releases its upload handle after the reply.
- Budget arithmetic recorded for reviewers: outstanding per-owner held objects ≤ 1 (released
  before dispatch) + 1 in-flight; the 256 KiB per-writing-task budget charges atad for read
  payloads ⇒ safe for ≤ 3 concurrent client connections in v1 (risk table §9).

### 4.6 Typed client (`ata-protocol/src/client.rs`, modeled on `vfs-protocol/src/client.rs`)

- `Ata::connect() -> Result<Ata, ClientError>` (50 × 100 ms retries, same constants as the
  vfs client).
- `identify(drive: u8) -> Result<DriveIdentity, ClientError>` with
  `DriveIdentity { model: String, firmware_rev: String, sectors: u64 }`.
- `read(drive: u8, lba: u64, sector_count: usize) -> Result<Vec<u8>, ClientError>` — chunks
  automatically across transactions at `MAX_SECTORS_PER_REQUEST`.
- `write(drive: u8, lba: u64, data: &[u8]) -> Result<(), ClientError>` — chunks to a multiple
  of 512 and ≤ 128 sectors per transaction; `Protocol` error on non-multiple length.
- `flush(drive: u8) -> Result<(), ClientError>`; `disconnect(self)`.
- `ClientError` mirrors `vfs_protocol::client::ClientError` shape (protocol / transport /
  no-connection / data / `Ata` variants).

---

## 5. Daemon Internals — `apps/atad`

Crate shape mirrors `vfsd`: a `lib` (all logic, host-testable) plus a thin `[[bin]]` with
`test = false` (`apps/vfsd/Cargo.toml:8-15` template). Deps: `system`, `usrlib`,
`ata-protocol` only.

### 5.1 Task-file engine (lib, `taskfile.rs`) — pure against a bus trait

The engine never calls syscalls directly; it is written against a small **bus trait**
(methods: 8-bit in, 8-bit out, block u16 in, block u16 out) so host unit tests script a fake
IDE device byte-for-byte (the `interrupt.rs` `MockController` precedent, fact 5). Operations,
all polling loops **bounded** (§5.4):

- **set_drive(drive):** out8 `0x1F6 = 0xA0 | (drive ? 0x10 : 0)`; then settle.
- **settle (the 400 ns rule):** four non-destructive alternate-status reads from `0x3F6`
  (the classic status-read delay trick; four reads ≫ 400 ns even on QEMU TCG). Issued after
  every task-file register write sequence. Never read `0x1F7` for delay (reading status has
  interrupt side effects; `0x3F6` is the non-destructive choice).
- **probe/identify(drive):** pre-check floating bus (in8 `0x1F7` == 0xFF ⇒ absent);
  set_drive; settle; wait BSY clear (in8 `0x3F6`, bit 7); out8 `0x1F7 = 0xEC`; settle;
  wait BSY clear with bounded timeout; signature check in8 `0x1F2 == 0x00` and in8 `0x1F5 ==
  0x01` (ATA; ATAPI `0x1F2 == 0x3C` ⇒ treat as NotFound in v1, it cannot take ATA commands);
  wait DRQ (bounded); block-read 256 words from `0x1F0`; parse words 27–46 (model), 23–26
  (firmware), 60–63 (u64 LBA count, clamped to `0x0FFF_FFFF` for LBA28); if the capacity
  words read as 0 and the device predates LBA, fall back to the CHS geometry product from
  words 1/3/6 (real pre-LBA quirk; QEMU always reports words 60–63).
- **read_sectors(drive, lba, count, buf):** out8 sequence `0x1F2 = count`,
  `0x1F3/0x1F4/0x1F5 = lba lo/mid/hi`, `0x1F6 = 0xE0 | (drive << 4)` (LBA mode + drive bit in
  the head nibble); settle; out8 `0x1F7 = 0x20`; then per sector: poll `0x3F6` until BSY
  clear, test ERR (0x01) ⇒ abort with the err-register snapshot, test DRQ (0x08), block-read
  256 u16 words from `0x1F0` into `buf[sector*256..]`.
- **write_sectors:** same register sequence, command `0x30`; per sector wait DRQ, block-write
  256 words, then poll BSY after each block (a status read between blocks is required to let
  the device re-assert DRQ cleanly on real hardware).
- **flush:** out8 `0x1F7 = 0xE7`; wait BSY clear; ERR check.
- Failure classification for the wire: BSY-timeout ⇒ `Busy`; ERR bit ⇒ `Device` (+ err/status
  bytes); `PortError::Denied` from any bus call ⇒ `Denied`; nothing else is invented.
- Capacity guard lives here (`BadAddress` when `lba + count > identified sectors` for a
  probed drive), because **domain validity is the driver's property** (AGENTS.md: domain
  knowledge is owned by the serving subsystem).

Host unit tests (lib, written with the engine before the bin is wired): scripted bus for
probe-absent (0xFF float), probe-ATA-ok (synthetic IDENTIFY block asserting byte-swapped
model decode), DRQ-per-sector read (two 512 B patterns), ERR-after-first-sector ⇒ Device with
snapshot, BSY-forever ⇒ Busy within bounded spin, write-sequencing log (exact byte order of
task-file writes), flush happy path.

### 5.2 Daemon bin (`main.rs`)

Modeled on `vfsd::VfsServer` (fact 9):
1. Print `[ATAD] ATA disk driver`.
2. Probe drives 0 and 1. For each present drive print
   `[ATAD] ata0 drive<N> ready: <model> sectors=<decimal>`; print
   `[ATAD] no port access` and park in a sleep-retry loop if the very first probe returns
   `Denied` (defensive; means boot wiring missing).
3. Outer loop: `Syscall::ipc_bind(wire::SERVICE_NAME)` with ps2d-style retry (only rebind if
   bind failed).
4. Inner loop: `ipc_accept_message` → `wait_for_message` → skip frames without a
   `connection()` → `release_held_for(owner)` → `wire::Request::decode` → dispatch to the
   cached drive engines → `ipc_send(connection, reply)`. Decoding never panics;
   malformed frames ⇒ `REPLY_ERROR(Protocol)`.
5. State: identities for the two drives, `held: Vec<(owner, UserDataHandle)>`
   (exact vfsd idiom).

### 5.3 End-to-end READ flow (numbered, crossing every boundary)

1. `ata` client: `Ata::read(0, 0, 1)` → `Request::Read.encode()` → `Syscall::ipc_send` trap.
2. Kernel (`IpcSendMessage` handler): mailbox delivers to the `"ATA"` service; the
   notification-driven future parks the client on `ipc_receive_message`.
3. atad: `wait_for_message` wakes with the envelope; decode READ; capacity guard passes.
4. atad task-file engine: `Syscall::port_out` sequence ending in `port_out(0x1F7, 0x20, 1)`.
5. Kernel `PortOut` handler: width/port valid → grant check on atad's `TaskHandle` passes →
   `kernel().port_io` (arch `X86_64PortIo`) executes the `outb 0x20, 0x1F7`.
6. QEMU's IDE device model latches the command; BSY asserts then clears; DRQ asserts when
   sector 0 is staged.
7. atad: `port_in(0x3F6, 1)` polls status (again via trap → arch `inb` → device); then
   `port_in_words(0x1F0, &mut buf[0..256])` → kernel `PortInBlock` → one trap, 256 `inw`.
8. atad: `Syscall::ipc_data_write(connection, &buf)` → kernel copies into a data object,
   returns token; atad pushes it onto `held`, replies `REPLY_DATA { token, sectors: 1 }`.
9. Client: `ipc_receive_message` + `wait_for_message` return the reply; drain via
   `ipc_data_read` chunks; `ipc_data_release`; returns the 512 bytes.

### 5.4 Polling vs notification (v1 decision)

v1 polls status/alternate-status in **bounded** loops: a bounded spin budget (order
10 000 polls; BSY is a device-internal delay, microseconds in QEMU), then one
`Syscall::sleep(1)` (timer future — notification-driven, costs no polling on the preemption
cycle) repeated up to a whole-operation budget (e.g. 250 ms for a 128-sector transfer), then
`Busy`. Rationale: single-core hobby OS; the MLFQ timer preempts the daemon mid-spin so
system liveness is never at risk; correctness is testable against the scripted mock without
any device. IRQ14 delivery (handler in arch `interrupts.rs` calling
`deliver_to_service("ATAEVENT", KERNEL_EVENT_CONN frame)` per the PS/2 precedent
`arch/x86_64/src/interrupts.rs:117`, with atad waiting on both its binding and its event
mailbox) is the **named v2 upgrade path** — explicitly not v1 because per-request polling is
already bounded and the event-driven loop would double atad's synchronization surface for
zero v1 benefit.

---

## 6. Milestones (each independently shippable; AGENTS.md "work incrementally")

Verification-gate labels: a gate is **green-verbatim** only when justified by a cited fact;
gates marked **`unverified`** must first run green in the orchestrator's baseline pass before
being treated as binding. No step depends on code introduced by a later step.

### M0 — Environment spike: prove QEMU exposes the boot disk on the primary IDE channel
- **Concept only, no driver code.** New `tests-integration/tests/ata_env_spike.rs`:
  `QemuSession::spawn()` → expect `"rose>"` (guest quiescent at the shell; nothing in RosX
  touches ATA yet, so bus poking is safe) → `Monitor::connect(session.temp_dir().join(
  "monitor.sock"))` (public API, fact 13) → HMP port commands: select drive
  (`o /b 0xa0 0x1f6`), issue IDENTIFY (`o /b 0xec 0x1f7`), bounded loop of `i /b 0x1f7`
  until BSY clears, signature check `i /b 0x1f2` / `i /b 0x1f5`, then read 256 words via
  `i /w 0x1f0` and reassemble the byte-swapped model string.
  **Machine-countable evidence:** the reassembled string contains `HARDDISK` (QEMU's default
  IDENTIFY model) — assert on it.
- **Dry-run launch gate first:** before writing assertions, manually issue the HMP commands
  against one throwaway session to pin the exact reply text format the parser will see.
- Fallback if the installed QEMU lacks HMP port commands (`i`/`o`): temporary kernel-side
  probe in `arch/x86_64/src/main.rs` (a few `inb`/`outb` + IDENTIFY print via `kprintln!`),
  run `cargo run`, read the banner, then **git-revert the probe** — the spike must not
  survive into the tree. Record which route worked in the commit body.
- **Gate:** the spike passes (`unverified`: single-test invocation
  `cargo test -p tests-integration --test ata_env_spike -- --test-threads=1`; the
  harness-proven umbrella `cargo xtask test` also runs it).
- **Commit:** `Probe QEMU IDE environment through the monitor before writing the driver`

### M1 — Port-I/O capability (treaty + kernel mechanism + arch impl + unit tests)
Files & changes, smallest compilable order:
1. `system/src/syscall_numbers.rs`: add `PortIn = 28`, `PortOut = 29`, `PortInBlock = 30`,
   `PortOutBlock = 31`; extend `TryFrom`; **extend the live-number array in
   `every_live_number_round_trips_through_try_from`** (fact 1). The retired-numbers test is
   untouched (4/9 stay invalid).
2. `system/src/error.rs`: add `PortAccessDenied = 0x0301`, `PortRequestInvalid = 0x0302`
   + `from_code` arms + grow `ALL_ERROR_CODES` to `[ErrorCode; 17]` (fact 8).
3. `kernel/src/portio.rs` (new) + `pub mod portio;` in `kernel/src/lib.rs`: `PortIo` trait;
   grant table (`KernelCell` static, 8 entries, keyed by full `TaskHandle`; `grant`,
   `revoke`, `authorize(task, port, width_or_block) -> bool`); the §3.3 validation rules
   written as a pure function of (grant table, request) so they are host-testable without
   arch code. Host tests (mock `PortIo` recording an op log, the `interrupt.rs` pattern):
   grant boundary inclusive both ends; width straddling a grant edge denied; 4-byte grant
   math; block op covered vs uncovered port; ungranted task denied; generation-keyed miss
   after simulated slot reuse; revoke; table-full failure; width 3 rejected; count 0 and
   32769 rejected; `port+width` wraparound rejected.
4. `kernel/src/kconfig.rs`: add `pub port_io: &'static dyn PortIo`; `kernel/src/kernel.rs`:
   store it (`pub(crate)` field like `elf_arch`).
5. `kernel/src/syscall.rs`: four `#[cfg(not(test))]` arms per §3.3 (authorize via
   `crate::portio::authorize(kernel().execution_state.current_task(), ...)`, then call
   `kernel().port_io`; `PortIn` writes through the arg3 out-pointer — see §3.3's aliasing
   warning). No changes to `arch/x86_64/src/syscall.S` (dispatch is the existing
   number-matched handler; the trap plumbing is number-agnostic).
6. `arch/x86_64/src/portio.rs` (new: `mod` decl + static `PORT_IO` + `KCONFIG` field):
   `in`/`out` asm. `arch/x86_32/src/portio.rs` (new, wired into its `KCONFIG` at
   `arch/x86_32/src/main.rs:17-21`): reuse the existing `inb`/`outb` idiom
   (`arch/x86_32/src/interrupts.rs:140,165`). **No grants wired yet** — deny-by-default.
   Both arch builds must compile at the end of this step; nothing in between is shipped.
7. `usrlib/src/syscall.rs` + `usrlib/src/error.rs`: wrappers + `PortError` (§3.6).
8. `AGENTS.md` sweep (this step): the kernel-tree overview line (listing
   `scheduler/ … cpu (HAL trait) kconfig rom/`) gains the `portio` capability entry; the
   x86_32 notes paragraph gains one clause mirroring the vfsd sentence: x86_32 wires the
   port mechanism but boots with **no grants and no atad**. No other AGENTS.md line states a
   fact this step invalidates (AGENTS.md enumerates no syscall list).
- **Blast radius / test processes:** both `system` treaty tests (facts 1, 8); every `KConfig`
  consumer — exactly the two arch `main.rs` (fact 4), proven by *both* arch builds below;
  the existing host suite via `cargo xtask test --skip-integration`; the boot path of all
  nine QEMU test processes (shared global state changes: new syscall dispatch arms + new
  KConfig field — no grant exists, so behavior is unchanged, but the QEMU process kind is
  still exercised via the full `cargo xtask test`).
- **Gates:** `cargo xtask test --skip-integration` (CI-parity; green today, fact 15, and
  stays green with the extended arrays) · `cargo xtask build` · x86_32 kernel build: from
  `arch/x86_32/`, `cargo build -p rosx-x86` — **`unverified`** (sourced from the AGENTS.md
  build section; no x86_32 build was executed while planning).
- **QEMU manual verification:** none possible yet (deny-by-default is proven by unit tests).
- **Commit:** `Add mediated x86 port I/O syscalls guarded by per-task port grants`

### M2 — `ata-protocol` crate (the external treaty)
- New `ata-protocol/` (`Cargo.toml` deps `system` + `usrlib` exactly like
  `vfs-protocol/Cargo.toml`), `src/lib.rs`, `src/wire.rs` (§4 constants, tag table, codec
  types `Request`/`Reply`, `AtaError`), `src/client.rs` (§4.6). Workspace `Cargo.toml`:
  add to `members` **and** `default-members` (mirroring the vfs-protocol/input-protocol
  lines).
- Host codec tests (copy `vfs-protocol` rigor level): offset pinning for every frame,
  byte-exact little-endian expectations, round-trip all variants, reject drive > 1, reject
  LBA bits 28–31 set, reject count 0 / count 129, reject unknown op/tag, zero-tail
  enforcement on REPLY_ACK, `AtaError` byte round-trip incl. unknown→`Protocol`,
  opcode/tag disjointness test, and the arithmetic test
  `MAX_SECTORS_PER_REQUEST * SECTOR_BYTES ==` data-object cap.
- **Gates:** `cargo test -p ata-protocol` (becomes valid when the package exists;
  **`unverified`** as a brand-new package) · `cargo xtask test --skip-integration`.
- **Commit:** `Add ata-protocol wire codec and typed client for the disk daemon treaty`

### M3 — `atad` minimal: probe + IDENTIFY over IPC, embedded at boot
- New `apps/atad/` (crate shape fact 11; deps `system`, `usrlib`, `ata-protocol`),
  `src/taskfile.rs` lib core **with the bus trait + probe/identify implementation and the
  §5.1 mock-device host tests** (probe-absent, probe-ok model decode, ERR/Busy paths),
  `src/main.rs` bin: boot banner + drive probe/report lines (§5.2 steps 1–2) + bind/accept
  loop serving **IDENTIFY**; READ/WRITE/FLUSH tags decode but answer
  `REPLY_ERROR(Protocol)` until their milestones — a replaceable seam that leaves no dead
  branches after M5.
- `arch/x86_64/src/main.rs`: embed + schedule (pattern fact 6) **before** `SHELL_ELF`,
  capture the `TaskHandle` from `kernel.schedule(...)`, and wire policy at the arch
  boundary: grant that handle `0x1F0–0x1F7` and `0x3F6` (two grants; `kprintln!` on
  grant-table failure). Ordering note for the coder: `include_bytes!` needs
  `target/rosx-user/release/atad` — run `cargo xtask apps` (or use `cargo xtask build`,
  which orders apps → rom → kernel) before any bare `cargo build -p rosx`.
- `xtask/src/main.rs`: `USER_APPS` `[&str; 16]` → `[&str; 17]` + `"atad"` (the array length
  is a literal — the compiler guards the update; the ROM roster name `atad` is
  underscore-free per the AGENTS.md constraint).
- New `tests-integration/tests/ata_identify.rs`: spawn → expect the two markers
  `[ATAD]` and `sectors=` within boot budget (proves boot embed, grant wiring, real
  IDENTIFY against QEMU's IDE device, and the print path) → expect `rose>`.
- `AGENTS.md` sweep (this step): the workspace-tree `apps/` inventory line gains `atad`; add
  one bullet under **Kernel subsystems (current invariants)** — disk architecture: command
  semantics live in user-space `atad` over `ata-protocol`; the kernel contributes only the
  mediated port capability; IRQ14 delivery named as the v2 event path.
- **Blast radius:** the boot path of every existing QEMU test grows one task + banner lines
  (needle-based tests, fact 14 — additions cannot break them, but run them anyway via full
  `cargo xtask test`); ROM roster +1 (shell `ls` shows `atad` — no test pins the roster
  size; verified by grepping all nine test files' needles).
- **Gates:** `cargo xtask test --skip-integration` · `cargo xtask build` · full
  `cargo xtask test` (includes `ata_identify`). Manual: `cargo run`, see the identity line.
- **Commit:** `Add atad user-space ATA daemon reporting drive identity`

### M4 — LBA28 READ over the data plane + `ata` demo client
- `apps/atad/src/taskfile.rs`: `read_sectors` + mock tests (per-sector DRQ, payload
  correctness, mid-stream ERR). `src/main.rs`: READ handler (engine → `ipc_data_write` →
  `REPLY_DATA`; held-object discipline §4.5).
- New `apps/ata/` workspace member (shape of `apps/mem`): connect to `ATA`; no-args mode
  prints identity of drive 0 (`ata model=<...> firmware=<...> sectors=<n>`), reads LBA 0 of
  drive 0, prints the MBR: signature line (`mbr signature 55AA ok`) and four partition lines
  (`mbr part<i> boot=<b> type=<t> lba=<lba> sectors=<n>`). Also an
  `ata read <drive> <lba> <sectors>` subcommand (checksum line only) for later milestones.
  Add to workspace `members`/`default-members` and `USER_APPS` `[&str; 17]` → `[&str; 18]`
  (ROM name `ata` — underscore-free).
- New `tests-integration/tests/ata_read.rs`: spawn → `rose>` → mark → `send_line("ata")` →
  expect `model=`, `55AA`, then prompt return. The boot disk is *the* IDE disk (fact 13 +
  M0), so LBA 0 is deterministic and its `0x55AA` boot signature at byte offset 510 is
  guaranteed by the disk-image format; assert on that needle, not on partition contents
  (the bootloader's MBR may leave entries zero).
- `AGENTS.md` sweep: the `apps/` inventory line gains `ata` (the same line edited in M3).
- **Gates:** `cargo xtask test --skip-integration` · `cargo xtask build` · full
  `cargo xtask test` (incl. `ata_read`). Manual: `cargo run`, shell `ata`.
- **Commit:** `Read ATA sectors through atad and demo them with the ata client`

### M5 — WRITE + FLUSH + scratch slave disk in the test harness
- `apps/atad/src/taskfile.rs`: `write_sectors`, `flush` + mock tests (write-sequencing log,
  DRQ-before-each-block, trailing BSY). `src/main.rs`: WRITE handler (drain token, size check
  payload == `count*512` else `REPLY_ERROR(Protocol)`), FLUSH handler.
- `ata` client: `ata write <drive> <lba>` subcommand — build an 8-sector deterministic
  pattern (`(lba*7 + i) & 0xFF`), write, read back, print `writeback ok drive=<d> lba=<l>`
  or `writeback mismatch at byte <n>`; uses the typed client's `write`/`flush`.
- `tests-integration/src/qemu.rs`: add `QemuSession::spawn_with_scratch_disk()` — creates a
  zeroed 1 MiB raw file in the session `temp_dir()` and appends
  `-drive if=ide,index=1,media=disk,format=raw,file=<scratch>` (primary **slave**: the drive
  bit in `0x1F6` only — **no grant or daemon port changes**, the compat range is shared).
  Default `spawn()` behavior untouched (fact 14 blast-radius rule).
- New `tests-integration/tests/ata_write.rs`: `spawn_with_scratch_disk` → `rose>` → expect
  the atad boot banner naming **two** ready drives → `send_line("ata write 1 0")` → expect
  `writeback ok drive=1` → prompt. (Drive 0 is never written.)
- Runner (`arch/x86_64-runner`) stays unchanged; for interactive manual write testing use the
  manual recipe: build the image, start QEMU by hand with the produced image plus
  `-hdb scratch.img`.
- **Gates:** `cargo xtask test --skip-integration` · `cargo xtask build` · full
  `cargo xtask test` (incl. `ata_write`; default-spawn tests unaffected by the additive
  harness change).
- **Commit:** `Write and flush ATA sectors against a scratch slave drive under QEMU`

### M6 — Stress & polish
- `ata stress` subcommand: sequential 4 × 128-sector reads across the boot disk with a
  checksum per chunk (`stress read ok chunks=4`), two write/read-back cycles on drive 1
  exercising the held-object release discipline (client releases each drained object
  explicitly; atad's per-owner held list never exceeds one entry — verified indirectly by
  completing without `Busy`/`Denied`).
- Host-side test pass: every `taskfile.rs` branch has a named test; a byte-flip sweep test in
  `ata-protocol` (flip every byte of a valid frame; decode must accept-or-reject without
  panic).
- Docs sweep (bound follow-ups): grep the repo for app inventories missing the new members —
  `grep -rnE "vfsd|ps2d|inputd" AGENTS.md README.md docs/todo.md` — and correct any app list
  the M3/M4 line edits missed; README/docs lines that merely under-report are recorded as
  follow-up items, never silently skipped. AGENTS.md itself must be fully correct after this
  milestone (its `apps/` line, kernel-tree line, disk-architecture bullet, and x86_32
  no-atad clause are owned by M1/M3/M4).
- **Gates:** full `cargo xtask test` · `cargo xtask build`.
- **Commit:** `Stress the ATA read path and tighten transfer error reporting`

### M7 — Future work (explicitly out of scope; each its own later plan)
- IRQ14 → `deliver_to_service` event delivery (PS/2 pattern; §5.4) making atad's wait
  fully notification-driven.
- LBA48 (IDENTIFY word 83 bit 10 gate; commands `0x24`/`0x34`; the 65536-count rule).
- PCI IDE controller discovery (BARs) replacing the hard-coded compat grant at boot.
- vfsd-on-ATA (block-device backend; then `rose> cat` from disk).
- SET MULTIPLE (mode 0x06), x86_32 atad boot wiring (grant + embed once the i686 user-ELF
  flow builds it), real-hardware validation (400 ns margins, 80-pin cable slave presence).
- Wire `portio::revoke` into task teardown once any granted task can die (follow-up recorded
  here so it is never silently lost).

### Verification matrix

| Command (cwd)                                                     | Proves                                                        | Status |
|-------------------------------------------------------------------|---------------|--------|
| `cargo xtask build` (root)                                        | apps + ROM + kernel + disk image compile for the rosx / rosx-user targets | green today (CI runs it, fact 15) |
| `cargo xtask test --skip-integration` (root)                      | all host unit tests incl. new portio/codec/taskfile suites    | green today (CI runs it, fact 15) |
| `cargo xtask test` (root)                                         | + all QEMU integration tests incl. `ata_env_spike`/`ata_identify`/`ata_read`/`ata_write` | needs `qemu-system-x86_64` on PATH (the harness reports its absence via the spawn panic text, `tests-integration/src/qemu.rs:58-60`); the new `ata_*` runs are **unverified** until they first go green in an implementing transcript |
| `cargo test -p kernel -- --test-threads=1` (root)                 | kernel unit tests alone (portio grant table, interrupt, ipc)  | pattern from AGENTS.md; green today |
| `cargo build -p rosx-x86` (cwd `arch/x86_32/`)                    | x86_32 kernel builds with the new KConfig field               | **unverified** (AGENTS.md recipe; orchestrator baseline pass) |
| `cargo test -p tests-integration --test ata_read -- --test-threads=1` (root) | single ata integration test | **unverified** single-test invocation; the umbrella `cargo xtask test` is the binding form |
| `cargo run` (root)                                                | interactive QEMU: identity banner, shell `ata`; write test needs a manually attached `-hdb scratch.img` | manual companion |

---

## 7. Acceptance Criteria (final; per-milestone gates subsume these)

1. `cargo xtask build` — exit 0.
2. `cargo xtask test --skip-integration` — exit 0 (includes the extended syscall-number
   round-trip test over numbers 28–31, the 17-variant error-code array, the portio grant
   tests, the ata-protocol codec suite, and every atad taskfile mock-device suite).
3. `cargo xtask test` — exit 0 with `qemu-system-x86_64` available; all pre-existing
   integration tests still green **and** the new `ata_*` tests green, each asserting
   machine-countable needles (model string, `55AA`, `writeback ok`). The `unverified`
   ata-specific runs become proven when they first go green in the implementing transcript.
4. `cargo build -p rosx-x86` from `arch/x86_32/` — **`unverified`** until the orchestrator's
   baseline pass proves the command itself green pre-change; the acceptance obligation is
   then "no new x86_32 regression relative to that baseline".
5. No warning gate is asserted: this plan records no proven-empty warning baseline, and an
   absolute-zero warning claim over a substrate without one is unsatisfiable by construction.

---

## 8. CI Files & Doc Updates the Plan Implies

- **`.github/workflows/ci.yml`:** **no edit required** — its two commands
  (`cargo xtask test --skip-integration`, `cargo xtask build`) auto-cover the new workspace
  members and `USER_APPS` entries. Recorded gap: `--skip-integration` means the `ata_*` QEMU
  tests never run in CI (same posture as the existing nine); adding an integration job is out
  of scope and should be a separate, deliberate change (QEMU availability on runners).
- **`.github/workflows/build-artifacts.yml`:** no edit required (dispatches
  `cargo xtask build`).
- **`AGENTS.md`** (the only agent-doc needing edits; `CLAUDE.md`/`GEMINI.md` include it,
  fact 16): kernel-tree line (`portio`, M1) · x86_32 no-atad clause (M1) · `apps/`
  inventory line (`atad` M3, `ata` M4) · disk-architecture bullet under Kernel subsystems
  (M3). Any further invalidated inventory lines found by the M6 grep are recorded follow-up
  items, never silently skipped.

---

## 9. Risks & Open Questions

| # | Risk / question | Mitigation owned by the plan |
|---|-----------------|------------------------------|
| 1 | **`-drive format=raw` default-interface assumption** — if a future QEMU defaults a non-IDE interface, the driver sees no device. | M0 spike is the gate: it fails loudly (needle timeout) before any driver code is committed. M3's boot banner repeats the proof continuously. |
| 2 | **Grant strictness vs hobby pragmatism**: boot-time grants are static; adding a second granted task needs an arch-main edit. | Accepted posture: policy at the arch boundary is strictly *stronger* than binding-race designs (which silently grant whoever binds first). Table capacity 8, explicit `kprintln!` on overflow. |
| 3 | **Handle-slot recycling**: a dead granted task's table entry could match a regenerated handle. | Authorization keys the **full** handle incl. generation (miss on regeneration); `revoke()` provided now; wiring revoke into task teardown recorded as M7 follow-up (atad never exits in v1). |
| 4 | **32-bit `PortIn` aliasing the error bit** (values ≥ 0x8000_0000 look like errors). | Out-pointer return design (§3.3) made normative, with the trap spelled out so a coder cannot "simplify" it back. |
| 5 | **Busy-wait vs notification-driven scheduler**: a BSY spin consumes quanta on single-core; sleeping per poll would overshoot microsecond-scale waits. | Bounded spin (≈10k polls) → `Syscall::sleep(1)` (timer-future driven, not preemption-polling) → operation budget → `Busy`. Mock-device tests pin the loop bounds. |
| 6 | **QEMU vs real-hardware settling**: the 4-alternate-status-read delay comfortably exceeds 400 ns in QEMU TCG, but real drives/cables vary. | Non-destructive `0x3F6` reads (spec-correct); budgets centralized in named constants so real-HW tuning is a one-line, arch-visible change. Real HW deferred (M7). |
| 7 | **Data-plane budget** (256 KiB per writing task, fact 3): atad's held read objects could exhaust the budget with many concurrent connections. | Per-owner release-before-dispatch (vfsd-proven idiom) caps at 1 held + 1 in-flight per owner; documented v1 limit of ≤ 3 concurrent connections; overflow surfaces as `Busy`, never a panic. |
| 8 | **Sector count width**: wire count is u8 (up to 255) but the data-object cap is 128 sectors. | `MAX_SECTORS_PER_REQUEST = 128` enforced in codec encode **and** decode, pinned by an arithmetic test (§4). |
| 9 | **ROM roster naming** (AGENTS.md underscore rule): `atad` and `ata` are underscore-free — compliant. `ata` could conceptually collide with a future builtin; the shell's builtins are fixed at five (`ls clear rose sleep random`), so there is no collision in v1. | Recorded; shell list untouched. |
| 10 | **Build-order trap**: `include_bytes!` of `target/rosx-user/release/atad` breaks a bare `cargo build -p rosx` on a clean tree. | M3 step text mandates `cargo xtask apps` before kernel builds (same pre-existing constraint as `rom.img` — extended, not introduced). |
| 11 | **Integration tests are invisible to CI** — ata regressions could land green in CI. | Accepted for v1 (matches today's posture for all nine integration tests); the CI-job proposal is recorded in §8. |
| 12 | **HMP `i`/`o` command availability and exact reply format** in M0. | M0 includes an explicit dry-run launch gate (poke a throwaway session first) and a fallback (temporary kernel-side probe + revert); budget: one session, no user-space dependency. |
| 13 | **Boot-disk writes** during M5 experiments corrupting the test image. | Tests write only to the scratch slave (drive 1); boot-image writes are not part of any test's contract. |

---

## 10. Interfaces Summary (what new code must expose)

- `system::syscall_numbers::SyscallNum::{PortIn = 28, PortOut = 29, PortInBlock = 30, PortOutBlock = 31}`.
- `system::error::ErrorCode::{PortAccessDenied = 0x0301, PortRequestInvalid = 0x0302}`.
- `kernel::portio::PortIo` (trait: 1/2/4-byte port read, 1/2/4-byte port write, u16 block
  read, u16 block write — all pre-validated inputs), `kernel::portio::grant(task, base, end) -> bool`,
  `kernel::portio::revoke(task)`.
- `kernel::kconfig::KConfig.port_io: &'static dyn PortIo` (both arch KCONFIG literals
  updated in the same commit that adds the field).
- `usrlib::syscall::Syscall::{port_in, port_out, port_in_words, port_out_words}`;
  `usrlib::error::PortError`.
- `ata_protocol::wire::{SERVICE_NAME = "ATA", FRAME_BYTES, SECTOR_BYTES, MAX_SECTORS_PER_REQUEST,
  IDENTIFY, READ, WRITE, FLUSH, REPLY_ACK, REPLY_ERROR, REPLY_IDENT, REPLY_DATA, AtaError,
  Request, Reply}` with encode/decode per the §4 tables.
- `ata_protocol::client::{Ata, DriveIdentity, ClientError}` per §4.6.
- `atad::taskfile::{Bus (trait), AtaDrive}` (probe/identify/read/write/flush with bounded
  poll budgets), consumed by the `atad` bin only.

*End of plan.*
