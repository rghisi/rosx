# ipd — User-Space IP Stack (ARP, IPv4, ICMP) — Implementation Plan

> **Status:** Plan (code starts at M2). Written 2026-11-17 on branch `ipd-design` (base `3f1d059` = `origin/main`).
> **Feature:** `ipd` — a user-space IP daemon above `ne2kd` owning ARP, IPv4 and ICMP, speaking an
> `ip-protocol` wire treaty with subscribe-and-push datagram delivery; a `Watch` amendment to the
> `net-protocol` (NE2K) wire so frame arrival is notification-driven end to end; the Ethernet/ARP
> codec extracted from `apps/arp` into `net-protocol` with new IPv4/ICMP codecs beside it; the `ip`
> CLI (status, config, neigh, ping); and QEMU integration proofs on slirp, guest pairs, and
> injected frames.
> **Method:** every repo fact below was re-verified against this working tree (paths cited).

**Decisions (owner-approved in session 2026-11-17):**
- v1 finish line: **ping works end to end on both arches**, with ipd as the ARP authority. UDP is the next plan, TCP its own.
- The `NE2K` wire **is amended** with a `Watch` RX subscription in this branch — a poll-based ipd is not acceptable (it fights the no-polling invariant and cannot join a waitset).
- The client face is **raw pushes**: ipd delivers datagrams (tagged, filtered by ICMP kind); clients parse ICMP themselves through `net-protocol`. No semantic per-opcode API growth here.
- Configuration is **strictly manual**: no boot plan step, no plan-grammar change, no DHCP. The `ip` CLI configures from the shell and auto-launches the daemon chain when bindings are absent.
- Layering: ipd is the single L3 authority; `"NE2K"` stays the vendor face for diagnostics (`arp`, `ne2k`) — the blockd-vs-`ata` relationship, norm not enforcement.
- Codec home: Ethernet+ARP move from `apps/arp` into `net-protocol` (the AGENTS.md note pre-decided this); IPv4/ICMP codecs join them. Daemon policy stays in `apps/ipd`.
- Naming: daemon `ipd`, binding `"IP"`, CLI `ip`. **`ping` is taken** by the IPC bulk demo (fact 4) — the ICMP client is the `ip ping` subcommand.

---

## 1. Goal & Non-Goals

### Goal
- v1 delivers on **both x86_64 and x86_32**: ARP resolve/answer/cache, IPv4 demux with header
  checksum verification, ICMP echo answer, echo-request-out via `ip ping`, direct-or-gateway
  forwarding on one interface, static address configuration.
- Fully notification-driven stack chain: NIC IRQ → `ne2kd` → `Watch` push → `ipd` → client push.
  No component polls for arrival; `arp watch`-style polling stays only where it is the honest
  vendor-face diagnostic.
- Kernel and `system` stay network-blind — the ne2000 plan's "kernel never sees frames" extends
  unchanged: the kernel never sees packets either.

### Non-Goals (each a named future plan)
- **UDP**: next plan on the same daemon; ipd's wire reserves its opcode band now (bind/send, port
  authority). **DHCP** rides UDP and is that plan's companion.
- **TCP**: its own plan. Recorded warning: TCP streaming over request/reply RPC + bounded data
  objects is the awkward fit in this architecture; that plan must decide between ipd-managed TCP
  state and a bulk channel mechanism — not smuggle it into ipd.
- **Fragmentation/reassembly**: refused honestly (payload > 1472 bytes → `Oversize`).
- **Broadcast/multicast addressing**, IPv6, multi-interface routing/forwarding between nets.
- **Boot plan / ROM**: `ipd` and `ip` are NOT in `BOOTSTRAP_APPS` and get NO plan step (manual
  doctrine). `pcid`/`ne2kd` plan steps stay as they are.
- **Kernel changes**: none. No timer future, no socket syscall, no connection-lifecycle change.
- **Real hardware evidence**: no hardware checklist item gains green from this branch.

---

## 2. Verified Baseline Facts (re-checked 2026-11-17)

1. **NE2K wire is poll-only on RX**: opcodes `MAC/SEND/POLL/STATS` = 0x01–0x04, reply tags
   0x10–0x15 (`protocol/net/src/wire.rs:7-17`); server holds a 32-frame RX queue
   (`apps/ne2kd/src/main.rs:25`) and its event loop is a waitset over the service accept + the
   `"IRQ<n>"` accept (`main.rs:196-229`); `on_irq_event` already drains the ring
   (`main.rs:304-321`) — the exact seat where a `Watch` fan-out belongs.
2. **Mailboxes are symmetric and errors are honest**: the daemon's reply path IS a plain
   `ipc_send` on the accepted connection; a server→client unsolicited push needs no new primitive.
   `push_back` distinguishes `ConnectionNotFound` (connection removed) from
   `ConnectionCongested` (mailbox full at `MAILBOX_DEPTH = 32`)
   (`kernel/src/ipc/mailbox_manager.rs:66-78`, `kernel/src/ipc/errors.rs:15-16`,
   `kernel/src/ipc/mailbox.rs:4`). A connection dies on the client's `ipc_disconnect`, so a later
   push answers `ConnectionNotFound` (`kernel/src/ipc/ipc_manager.rs:118-127`) — the watcher-prune
   signal.
3. **No userspace timer future**: waitsets attach IPC futures only
   (`usrlib/src/syscall.rs:242-280`); `Sleep` is a direct sleep; wall time is `Syscall::now()`.
   Consequences pinned in design: no `recv` timeout (client-side deadline loops around a blocking
   receive are out; a client either blocks or subscribes), and ipd/arp-cache aging rides each wake.
4. **The name `ping` is taken** by the IPC bulk-data demo (`xtask/src/arch.rs:5` `USER_APPS`,
   boot-plan step `.service("/pong", "PING")` in `arch/x86_32/src/main.rs:70`,
   `apps/ping/src/main.rs`). The ICMP client is `ip ping`; the demo is untouched.
5. **Extraction-ready codec**: `apps/arp/src/arp.rs` is dependency-free, host-tested Ethernet+ARP
   codec; AGENTS.md marks it "extraction-ready for `net-protocol`". `apps/arp/src/main.rs:236-264`
   holds the proven daemon auto-launch pattern (`ensure_network_stack`).
6. **Boot plan grammar and bounds**: `.service(program, probe)` carries no argv
   (`arch/x86_32/src/main.rs:60-74`); the cap is `MAX_PLAN_STEPS = 15` (`system/src/boot_plan.rs:11`)
   and both ports ship 14 steps — v1 adds none.
7. **QEMU media exist**: slirp PCI attach (`arch/x86-runner/src/lib.rs:238-240`) and mcast ISA
   attach (`lib.rs:229-236`), wired by the harness (`tests-integration/src/qemu.rs:144,160`);
   slirp's proxy-MAC ARP is already proven by `tests-integration/tests/arp_slirp.rs`, and frame
   injection into the mcast group by `arp_watch.rs`.
8. **Data-family patterns**: write→token→chunked-read→release is proven on both sides of a frame
   pipe (`protocol/net/src/client.rs:23-41`, `apps/ne2kd/src/main.rs:137-147`); pushed payloads
   reuse it unchanged, with the receiver releasing.
9. **Mockable-driver precedent**: `dp8390.rs` over `trait NicBus` with a transcript-asserting
   `Ne2000Sim` (`apps/ne2kd/src/dp8390.rs`) is the shape ipd's core copies one layer up.
10. **486 discipline holds unchanged**: no FP, atomics ≤ 32 bits, `cargo xtask check-isa x86_32`
    audits the new ELFs; ipd is a single-task daemon, so it needs no atomics at all.

---

## 3. Architecture

```
apps:  ip ping / servers          clients of "IP", parse ICMP via net-protocol
ipd:   arp cache · ipv4 demux · icmp · one default route      binds "IP"
net:   net-protocol::ethernet|arp|ipv4|icmp    (inter-machine codecs, host-tested)
ne2kd: DP8390 truth, Watch fan-out + poll queue                 binds "NE2K"
kernel: mailbox + data family + IRQ events — network-blind
```

The two process-boundary treaties keep their established shapes: 60-byte tagged control frames,
bulk payloads on the IPC data family, typed clients with retry-connect, opcode/reply/push byte
bands with reservations.

**`net-protocol` additions (NE2K face).** `Watch = 0x05` subscribes the sending connection for
frame pushes — once per connection, frames are duplicated to watchers AND the poll queue, so every
existing poll client (`arp`, `ne2k`) keeps byte-identical semantics. Push tag `PushFrame { token,
length } = 0x20` (fresh band above the replies). A push is a data-object write plus the tagged
message; send failure releases the object and counts (`ConnectionCongested` = drop,
`ConnectionNotFound` = drop + prune watcher). `Stats` gains `watch_pushes`/`watch_drops` — codec,
server, client and `ne2k status` all move in the same commit, everything is in-tree.

**`ip-protocol` (new, `protocol/ip`, package `ip-protocol`).** `SERVICE_NAME = "IP"`.

| Direction | Message | Carriage |
|---|---|---|
| client→ipd | `Status` | reply: link, configured, addr/prefix/gw, mac, counters |
| client→ipd | `Config { addr, prefix_len, gw }` | Ack \| Err; unconfigured ipd serves only `Status` |
| client→ipd | `Neigh` | fixed-stride cache blob on the data family (the `pci List` precedent) |
| client→ipd | `Subscribe { icmp_kind }` | Ack; pushes start on this connection (kind byte, 255 = all ICMP) |
| client→ipd | `SendIp { dst, proto, length }` | data family; fire-and-forget, no ack |
| ipd→client | `Datagram { src, proto, length }` (push) | data family |
| ipd→client | `Unreachable { dst, reason }` (push) | async error band |

TX is one-way so a subscribed connection carries almost purely inbound traffic; the typed client
demultiplexes by tag. Errors: `NotConfigured`, `NoRoute`, `Oversize`, `Congested`, `Protocol` —
byte round-trip tests in house style. UDP (`0x10`-band) and TCP bands stay reserved from day one.

**ipd core (`apps/ipd` lib module, the `atad`/`ne2kd` placement).** `trait FrameDevice
{ transmit, receive, mac }` with a scripted `MockDevice`; every clock reading enters as a `now_ms`
parameter — the core never calls syscalls, so the whole ARP/ICMP machine is host-proven by exact
frame transcripts (resolve-then-echo sequence, gateway MAC choice, cache aging, concurrent-resolve
coalescing, drop accounting) before any VM boots. The bin is thin glue: retry-connect to `"NE2K"`,
`Watch`, one waitset over the client accept + the NE2K receive future, `Syscall::now()` fed into
aging at every wake. Unconfigured-until-`Config` is the whole boot story: no NIC at all exits with
a failure status like `ne2kd` does.

**The `ip` CLI.** `status` · `config <a.b.c.d>[/len] [gw a.b.c.d]` · `neigh` ·
`ping <a.b.c.d> [count] [size]`. Auto-launches `pcid`, `ne2kd`, `ipd` detached when their bindings
are absent (fact 5's proven pattern). `ping` builds/parses ICMP through `net-protocol`, measures
RTT with `Syscall::now()`, prints Unix-shaped lines and a summary.

---

## 4. Milestones (one commit each, gate green before the next)

**M1 — this document.** Gate: review.

**M2 — `net-protocol` packet modules.** `ethernet.rs` + `arp.rs` move (logic byte-identical,
tests ride along) out of `apps/arp/src/arp.rs`; `apps/arp` switches to `net_protocol::arp` with
zero CLI-visible change — the `arp_slirp`/`arp_pair`/`arp_watch` suites stay green unmodified.
New `ipv4.rs`: total header parser (version/IHL/lengths/protocol/addrs/header-checksum), honest
refusals for foreign versions and truncation, minimal encoder. New `icmp.rs`: echo request/reply
encode + total decode with message checksum. House test bar: pinned wire images, round-trips,
bit-flip decode-never-panics. Gate: `cargo test -p net-protocol -p arp`, arp integration suites.

**M3 — the `Watch` amendment.** Wire: opcode + `PushFrame` tag + the two `Stats` counters.
Server: watcher list, fan-out inside the existing event path, release-on-send-error, prune on
`ConnectionNotFound`, drop accounting. Client: `watch()` + a blocking `receive_frame()`; a new
`ne2k watch [count]` subcommand receives pushes (blocking — there is no timeout future, so the
harness supplies the frames, the `arp_watch` injection pattern) — the first in-tree proof of
daemon→client unsolicited push (fact 2: the kernel already supports it; nobody has used it). Poll
and every existing client untouched. Gate: `cargo test -p net-protocol -p ne2kd -p ne2k`,
`ne2k_smoke`/`ne2k_pair`/arp suites.

**M4 — ipd stack core as a mock-tested lib.** `FrameDevice` + `MockDevice`; `arp_cache`
(learn-from-any-frame, `now_ms`-driven aging with bounded TTL, resolve with bounded retries and a
capped pending queue that coalesces concurrent waiters on one who-has); `stack` (ethertype demux →
ARP answer/learn, IPv4 checksum verify + dest filter + own-bucket, ICMP echo answer, the
outbound direct-or-gateway path with resolve-before-send and the 1472-byte refusal). Full
transcript tests per §3. Gate: `cargo test -p ipd`.

**M5 — `protocol/ip` crate + ipd bin + `ip` CLI.** Wire treaty + typed client per §3. Bin per §3
(retry-connect NE2K, Watch, waitset, aging-on-wake, bind `"IP"`, rebind loop). `ip` CLI with the
auto-launch chain. xtask roster wiring (apps list, FAT disk — 8.3-clean names, no ROM). Gate: unit
tests, both arch builds, `cargo xtask check-isa x86_32`, boot-to-shell smoke.

**M6 — integration proofs.** `ip_slirp`: single guest on the slirp PCI attach — `ip config
10.0.2.15/24 gw 10.0.2.2`, `arp who-has 10.0.2.2` (L2 truth), `ip ping 10.0.2.2` (L3 round trip
against QEMU's slirp responder — the first in-tree evidence of that behavior, see risks).
`ip_pair`: two guests on the mcast medium — both configured, ping across, `neigh` populated on
both, and one side's `arp watch` witnesses the other side's ipd answering ARP (cross-stack proof).
`ip_inject`: host-injected frames into the mcast group (the `arp_watch` harness mechanism) —
crafted who-has ⇒ asserted is-at bytes; crafted echo-request ⇒ asserted echo-reply. Both arches
via `ROSX_ARCH`. Gate: full `cargo xtask test`, both arches, `check-isa`.

---

## 5. Risks

- **Unsolicited server→client push is a first in-tree.** The kernel supports it symmetrically
  (fact 2) but no daemon has used it. M3 lands `ne2k watch` as the isolated first proof *before*
  ipd depends on the mechanism.
- **Dead-client bookkeeping**: connections are reclaimed on explicit `ipc_disconnect` only
  (fact 2); a client that dies silently fills its 32-deep mailbox and then takes counted drops,
  and prune-on-error covers disconnect. This is the shape every existing daemon already inherits
  (`ne2kd`'s `held` list included); no kernel change is smuggled into this branch.
- **slirp ICMP echo behavior is an expectation, not in-tree evidence.** If QEMU's slirp refuses to
  answer `10.0.2.2` pings on some target, `ip_pair` stays the primary E2E proof and `ip_slirp`
  degrades to config + neigh + injection coverage without blocking the branch.
- **`Stats` layout change** touches codec + server + client + CLI together — small, wholly
  in-tree, one commit.
- **mcast pair flakiness** — inherited from the ne2000 plan's recorded risk; same degradation
  ladder (injected-frame proofs remain).
- **Name pressure on `"IP"` and `ping`**: the binding string `"IP"` is new and collision-checked
  against every `SERVICE_NAME`/`SOURCE_NAMES` roster; the ICMP client lives under `ip ping` because
  the `ping` name is the IPC demo's (fact 4) — renaming the demo is not in scope.
