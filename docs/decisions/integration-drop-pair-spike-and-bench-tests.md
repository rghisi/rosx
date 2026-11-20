---
status: decided
date: 2026-11-20
revisit-when: ipd or ne2kd gain a deterministic loopback or in-process test seam; or a CI machine is dedicated to one integration session at a time
---

# Drop the multicast pair tests, the ata-env spike, and bench_alloc from the integration suite

## Question

The integration suite is the flakiest job in CI, and several of its tests need
two live VMs talking over QEMU's user-mode multicast medium, or exist only to
print diagnostics or timing numbers. Which tests earn their wall time and their
flake exposure?

## Chosen

Remove five tests (analysis in
`plans/integration-flakiness-assessment-2026-11-20.md`):

- `tests-integration/tests/ip_pair.rs`, `arp_pair.rs`, `ne2k_pair.rs` — the
  two-VM multicast pair tests. Every session pays two full boots (~20 s floor)
  and a 10 s init probe of the absent `ne2kd` service, and the medium itself is
  lossy when the runner is contended: a shared host multicast tap drops frames
  under load, so "0% packet loss" assertions are not testable infrastructure
  guarantees, they are bets. Under injected CPU load the pair tests failed and
  their single-VM counterparts (`arp_slirp`, `ip_slirp`, `ne2k_smoke`) stayed
  green. The same product surface — ne2kd RX, ipd ICMP/ARP over a real medium —
  is asserted by `ip_slirp`, `arp_slirp`, `arp_watch`, `ip_inject` and
  `ne2k_smoke` without the pair topology.
- `tests-integration/tests/ata_env_spike.rs` — a debug spike: ~40 register
  dumps, no assertions, its own 10x500 ms environment scan, and it runs in
  every job even though `ata_identify`, `ata_read`, `ata_write`, `ata_stress`
  and `fat_partition_flow` already watch for a vanished or misbehaving drive
  with real assertions.
- `tests-integration/tests/bench_alloc.rs` — prints allocation numbers nobody
  asserts on and runs the allocation load twice. Throughput measurements belong
  to a perf job with thresholds, not to the correctness suite.

## Alternatives

- Keep the pair tests and retry them until green — masks rather than bounds
  the flake rate, and retrying burns minutes of QEMU wall time per red.
- Keep the pair topology but assert only liveness (a `ping` that eventually
  gets *any* answer) — considered; the surviving single-VM slirp tests already
  cover that weaker claim at one fourth the cost.
- Move the pair tests behind an ignored-by-default flag — rejected: an ignored
  test rots silently, and the repo rule is that production surface without a
  caller is deleted, not parked.

## Consequences

- Multicast-to-multicast QEMU wiring is no longer exercised by CI. If ipd
  multicast behaviour itself must be covered again, it needs a deterministic
  seam (in-process L2 loopback in tests), not a second VM.
- Allocation throughput is no longer tracked; if a number is wanted, add a
  perf job with an explicit threshold.
- The remaining flake classes (silent ATA probe failures, harness keystroke
  windowing, the init probe tax) are untouched by this decision and are
  tracked in the plan file's recommendations.
