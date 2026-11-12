# NE2000 (DP8390) Network Driver in User Space — Implementation Plan

> **Status:** Plan (code starts at M2). Written 2026-11-12 on branch `ne2000-driver` (base `adbcedf` = `origin/main`).
> **Feature:** `ne2kd` — a user-space NE2000 (DP8390-core, incl. Realtek RTL8029AS) Ethernet daemon
> speaking a `net-protocol` wire treaty, a `pcid` PCI enumeration daemon speaking a `pci-protocol`
> treaty, kernel-side IRQ-to-userspace event delivery, port-pool grants for NE2000 and PCI config
> space, the `ne2k` and `pci` CLIs, and a two-VM QEMU multicast integration proof.
> **Method:** every repo fact below was re-verified against this working tree (paths cited).

**Decisions (owner-approved in session 2026-11-12):**
- Userspace driver daemon, not a ring-0 driver — the `atad`/`ps2d` norm.
- Real IRQ delivery is in scope for v1 (the ATA plan's "IRQ is v2" is superseded for this device).
- PCI enumeration (8029AS discovery) is in scope for v1, as a `pcid` provider daemon.
- QEMU wiring + integration test are in scope for this branch.
- Driver core lives in `apps/ne2kd` as a lib module — the `atad` placement, no new top-level theme.
- Naming: daemon `ne2kd`, binding `"NE2K"`, CLI `ne2k`.

---

## 1. Goal & Non-Goals

### Goal
- v1 delivers on **both x86_64 and x86_32**: DP8390 init (halt/reset, DCR, ring, CURR, PAR/MAC,
  RCR/TCR, IMR, start), TX of one Ethernet frame per request via Remote DMA write, RX via
  Remote DMA read with ring wrap-around split, event-driven RX wake-up via the new IRQ event path.
- ISA variant: fixed base `0x300`, IRQ `9`. PCI variant: discovered through `pcid`
  (vendor `0x10EC`, device `0x8029`) → BAR0 + Interrupt Line.
- Kernel contributions stay minimal and portable: an `IrqSubscribe` syscall over the existing
  platform interrupt dispatch core, and new grantable pool ranges. No NIC knowledge in `kernel/`.

### Non-Goals (each a named future plan)
- **TCP/IP stack**: none. The daemon moves Ethernet frames; `system`/`kernel` never see them.
- **Full PCI bus scan / resource arbitration**: `pcid` v1 scans bus 0 only and reads
  vendor/device/BAR0/Interrupt Line/revision; no bridge walking, no BAR sizing, no assignment.
- **IRQ release/sharing table**: `IrqSubscribe` is idempotent per task; v1 never masks (an
  unmasked IRQ with no listener delivers into a dead mailbox and is dropped). Sharing policy,
  masked-on-exit, and APIC pin introspection are v2.
- **Boot plan / ROM**: `ne2kd`, `pcid` and the CLIs are NOT in `BOOTSTRAP_APPS` and NOT plan
  steps; they launch from the disk shell like `ata`. ROM stays lean.
- **DMA (bus master)**, full-duplex multi-TX buffering, `ioctl`-style flow control, VLANs, jumbo frames.
- **Real hardware evidence**: the hardware checklist gains no green items from this branch.

---

## 2. Verified Baseline Facts (re-checked 2026-11-12)

1. **Port access is grant-mediated.** `PortAcquire = 38` / `PortRelease = 39` are the highest live
   syscall numbers (`system/src/syscall_numbers.rs:34-35`); the kernel's `GrantTable` (capacity 8)
   checks every access against per-task grants (`kernel/src/platform/portio.rs`). The pool is the
   arch's `KConfig.port_pool` (`arch/x86_32/src/main.rs:19`, `arch/x86_64/src/main.rs:47`), today
   only `ATA_PORT_POOL` = 0x1F0–0x1F7 + 0x3F6 (`arch/x86-generic/src/port.rs:4-7`).
2. **Userspace PIO wrappers exist**: `Syscall::port_in/port_out/port_in_words/port_out_words`
   (`usrlib/src/syscall.rs:306-374`). Raw `in`/`out` asm exists only in `arch/x86-generic` and the
   arch interrupt code — a userspace driver cannot and must not execute them.
3. **Kernel→userspace event delivery precedent (keyboard)**: the IRQ1 handler builds a
   `system::ipc::Message` with `KERNEL_EVENT_CONN` and calls
   `kernel::ipc::deliver_to_service(SOURCE_NAME_PS2KEYBOARD, frame)`
   (`arch/x86_32/src/interrupts.rs:165-173`, `arch/x86_64/src/interrupts.rs:111-119`); ps2d binds
   the source name and blocks on `ipc_accept_message` — no polling anywhere.
4. **Portable interrupt dispatch core exists** (`kernel/src/platform/interrupt.rs`):
   `install::<C: InterruptController>`, `register_handler`, `dispatch_irq`, `unmask`. x86_64 uses
   it for timer+keyboard on both PIC and APIC (`arch/x86_64/src/interrupts.rs:61-67`); x86_32 has
   NOT installed a controller — its handlers are raw fixed IDT entries
   (`arch/x86_32/src/interrupts.rs:84-124`). Installing an x86_32 `InterruptController` is part of M2.
5. **Driver-in-app-lib precedent**: `atad` puts the device core in `apps/atad/src/lib.rs` modules
   over a `Bus` trait (`apps/atad/src/taskfile.rs:45-51`) and unit-tests it with a recording
   `MockBus` (`taskfile.rs:317+`); the bin wires the trait to syscalls.
6. **Wire treaty precedent**: `ata-protocol` (`protocol/ata`) — 60-byte frames, `SERVICE_NAME`,
   opcode/reply bytes, error enum with byte round-trip, typed client; bulk payloads ride the IPC
   data family (tokens), never the message frame.
7. **No PCI support exists anywhere** (grep for `pci` finds nothing) and QEMU args are built in
   `x86_runner::qemu_command` (`arch/x86-runner/src/lib.rs:192`), consumed by both
   `cargo run` and the integration harness (`tests-integration/src/qemu.rs:74`).
8. **Grant capacity**: `GRANT_CAPACITY = 8` (`kernel/src/platform/portio.rs:35`); atad holds 2,
   pcid will hold 1, ne2kd 1 → 4 total, inside the table.

---

## 3. Deviations from the original prompt (and why)

| Prompt | RosX adaptation |
|---|---|
| Ring 0, IRQ handler calls `receive_packet()` | Userspace `ne2kd` daemon; the kernel IRQ handler delivers an opaque *IRQ event frame* to binding `"IRQ<n>"` (keyboard precedent); the daemon drains the ring on that event |
| `inb/outb/inw/outw` directly | `Syscall::port_*` behind a `NicBus` trait (mockable), mirroring `atad::taskfile::Bus` |
| No MAC handling | **Gap fixed**: RDMA 16 B from SRAM 0x0000 (93C46 image, every byte duplicated), MAC taken as the low byte of words 0–5; PAR programmed on **page 1** (CR `0x61`, offsets `0x01–0x06` — page 2 is the multicast MAR); RCR gets the AR bit (`0x70` = AR+AM+AB, not `0x0C`) — without this the card accepts no unicast, ever. A read-back verify guards an empty PROM with a locally-administered fallback MAC |
| `outw` loop over `data` | **Gap fixed**: odd-length frames zero-pad the final `outw`; `TBCR` carries the true length |
| RX header status ignored | **Gap fixed**: header must carry PRX (`0x01`) with the CRC/alignment/FIFO-overrun/missed bits (`0x1C`) clear, else the frame is dropped (still advances BNRY); length is sanity-checked (total 64–1518) |
| ISR bit claims (prompt: reset bit 6, RDC bit 7) | **Prompt wrong, datasheet right**: ISR reset-complete is bit 7 (`0x80`), remote-DMA-complete bit 6 (`0x40`), TX error bit 3 (`0x08`); QEMU `ne2000.c` (`ENISR_RDC 0x40`, `ENISR_RESET 0x80`) confirms; unit-test simulators pin these |
| No overrun path | **Gap fixed**: ISR OVW re-programs BNRY/CURR (re-init RX ring) and reports via stats line |
| TX with no busy guard | **Gap fixed**: refuse TX while `CR.TX` is still set; read `TSR` after completion and surface `TCR.Undo` retransmit-free success honestly |
| Inline comments | Project rule "no comments, ever" → self-documenting constants (`CR_START`, `ISR_PACKET_RECEIVED`, `PSTART_PAGE`, …); this document carries the prose |
| IRQ number just passed in | `IrqSubscribe` syscall: unmask + handler registration; the IRQ number becomes a kernel-claimable resource on the event binding |
| PCI enumerator "already exists" | It does not (fact 7) → `pcid` provider daemon + `pci-protocol`, bus-0 scan |

---

## 4. Milestones (one commit each, gate green before the next)

**M1 — this document.** Gate: review.

**M2 — IRQ event path (kernel + both arches).**
- `system/src/irq.rs`: binding-name roster `SOURCE_NAMES: [&str; 16]` ("IRQ0".."IRQ15"),
  frame codec `encode_irq_event(irq) -> [u8; FRAME_BYTES]` / `irq_of(&frame)`, unit tests.
- `SyscallNum::IrqSubscribe = 40` (+ round-trip tests); handler in `kernel/src/syscall.rs` calls
  the dispatch core: `register_handler(irq, irq_event_handler)` + `unmask(irq)`; the handler
  delivers `system::irq` frames via `deliver_to_service(SOURCE_NAMES[irq], ..)`. IRQ 0/1/2/8 are
  refused (timer/cascade own vectors).
- `usrlib`: `Syscall::irq_subscribe(irq)`.
- x86_32: new `PicController` implementing the platform `InterruptController` over the existing
  raw PIC init, `interrupt::install::<PicController>()`, generic IDT entries for the master 3–7
  and all slave vectors 8–15 calling `dispatch_irq`; timer/keyboard handlers untouched.
- x86_64: IDT entries for master 3–7 + slave 8–15 → `dispatch_irq` (PIC and APIC both work
  through the dispatch core already).
- Gates: `cargo test -p kernel -- --test-threads=1`, `cargo test -p system`, both arch builds,
  `cargo xtask check-isa x86_32`, no `[IRQGUARD]` noise at boot.

**M3 — `protocol/net` crate** (`package net-protocol`, modeled on `protocol/ata`):
`SERVICE_NAME = "NE2K"`, 60-byte frames; ops `Mac` → `Mac[6]`, `Send { token }` → `Ack`,
`Poll` → `Data { token, len }` | `NoFrame`, `Status` → ring counters + mac; `NetError`
(`NoDevice`, `Busy`, `Oversize`, `Undersize`, `Device`, `Protocol`); typed client module. Unit tests.

**M4 — `protocol/pci` crate** (`package pci-protocol`): `SERVICE_NAME = "PCI"`;
`Find { vendor, device }` → `Slot { bus, device, function, vendor, device_id, bar0, irq }` |
`Missing`; `List { bus }` → data-family blob (fixed stride, `system`-style encode/decode helpers
live in the protocol crate — it owns its contract). Unit tests.

**M5 — `apps/ne2kd` lib: the DP8390 core** (`src/dp8390.rs` over `trait NicBus { read_byte,
write_byte, read_words, write_words, sleep_ms }` — same shape as atad's `Bus`):
`probe` (reset-RST poll + signature read), `init` (prompt sequence + PAR/RCR fixes + remote-read
of the 93C46 MAC), `send` (remote write, odd pad, RMT poll, TPSR/TBCR, `CR=0x26`, TSR check),
`receive` (header RDMA, status/length guards, payload RDMA with the `0x8000 → 0x4600` wrap split,
RDMA abort, `BNRY = next_page − 1` with `0x46` wrap), `pump` (drain ring while `BNRY != CURR`,
bounded queue). Constants module for every CR/ISR/DCR/RCR/TCR bit. `MockBus` unit tests asserting
the exact port-write transcript for init, TX (even and odd), RX straight, **RX wrap split**,
overrun recovery, and MAC fallback. Gate: `cargo test -p ne2kd`.

**M6 — port pool.** `arch/x86-generic`: `NE2K_PORT_POOL` = 0x280–0x3FF (all ISA slots incl. 0x300)
+ `PCI_PORT_POOL` = 0xCF8–0xCFB + `PCI_IO_BAR_POOL` = 0xC000–0xFFFF; both arches wire the
concatenation. `ne2kd` acquires `bar..bar+0x1f`; `pcid` acquires `0xCF8–0xCFB`.

**M7 — apps.** `apps/pcid` (lib: bus-0 scan over a `CfgBus` trait + mock tests; bin: acquire,
bind `"PCI"`, serve Find/List). `apps/ne2kd` bin: acquire (PCI-via-pcid, fallback ISA
0x300/9), init, `irq_subscribe`, waitset over the `"NE2K"` accept + `"IRQ<n>"` accept, serve Mac/
Send/Poll/Status, frames on the IPC data family. `apps/ne2k` CLI (`status`, `mac`, `send <hex>`),
`apps/pci` CLI (`find <vid> <did>`, `list`). xtask workspace/roster wiring (apps list, no ROM).

**M8 — QEMU proof.** `x86_runner::qemu_command` gains an opt-in net medium (env-gated or new
`LaunchMode` input — never default, existing arg-list tests must stay green):
`-netdev socket,mcast=230.0.0.1:<port>,id=net0 -device ne2k_isa,netdev=net0,iobase=0x300,irq=9`.
Integration test `ne2000` (x86_32 first, `#[ignore]`d like the benchmark suites only if the
multicast pair proves flaky): boot two linked sessions, `pcid`+`ne2kd` on both, `ne2k send` on A,
assert B prints the received frame; also asserts `pci find 10ec 8029` on a `ne2k_pci` session
variant. Gates: full `cargo xtask test`, both arches, `check-isa`.

---

## 5. Risks

- **APIC path (x86_64)**: IOAPIC pin 9 routing for `ne2k_isa` under the local APIC — unmask goes
  through `LocalApic`; if the pin needs explicit configuration, M2's `configure` hook is the seat.
- **NE1000 phantom first-byte** on real ISA silicon: QEMU's 8029 model is clean; the mitigation
  (dummy `inw` before each remote-DMA read) is specced as a one-line guarded quirk in M5.
- **`GRANT_CAPACITY`**: at 4 of 8 used with this branch; a future device will need the table grown
  — not this branch's problem, recorded here.
- **Multicast integration pair**: two QEMU processes on one host mcast group; if the sandbox
  blocks it, the test degrades to TX-only proof (TSR success) and the RX wrap split stays
  proven by M5 mock transcripts.
