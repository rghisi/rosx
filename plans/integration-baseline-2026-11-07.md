# Integration Test Baseline — Phase 0

> **Date:** 2026-11-07
> **Commit:** `3fb9690` (origin/main)
> **Purpose:** measured decomposition of `cargo test -p tests-integration` wall time, as the
> reference for the integration-suite speed-up plan (single-session grouping, parallelism,
> tiering, CI coverage for x86_64).

## Method

- Unmodified suite, run serially exactly as CI runs it today:
  `cargo test -p tests-integration -- --test-threads=1 -Zunstable-options --report-time`
- `ROSX_ARCH=x86_32` prefixed for the x86_32 runs; dev profile; kernel built by the
  harness `OnceLock` path (`ROSX_KERNEL_ELF` unset) for the dev baselines.
- Boot-to-prompt distribution from `tests/measure_boot.rs` (new, `#[ignore]`d, run with
  `-- --ignored --nocapture`, 5 samples): timestamps `spawn()` return (boot-image creation +
  QEMU exec), `[KERNEL] Initializing`, `[KERNEL] Starting`, and the `rose>` prompt,
  all from one start instant.

## Measurement host

4 cores / 7.2 GiB RAM, QEMU 10.2.1 pure TCG, no KVM exposure during the run. GitHub
runners differ (also 4 cores, `ubuntu-latest`); **ratios transfer better than absolute
seconds** — TCG speed and host noise differ.

## Boot-to-prompt distribution

| Stamp (from spawn) | x86_64 dev | x86_64 release kernel | x86_32 dev |
|---|---|---|---|
| session spawn (image creation + exec) | 0.20–0.47 s | **0.03 s** | ~1–2 ms |
| `[KERNEL] Initializing` | 14.6–15.2 s | **3.81 s** | 0.18 s |
| `[KERNEL] Starting` | 14.7–15.4 s | 3.83 s | 0.43 s |
| `rose>` prompt | 15.0–15.6 s | **3.88 s** | 0.81 s |

**Findings:**

1. On x86_64 **~97 % of the 15 s boot happens before the kernel prints its first banner**
   (init→prompt is only ~0.4 s). The cost is the pre-kernel path: SeaBIOS + the
   `bootloader` crate stage2 ferrying the boot image through emulated IDE PIO under TCG.
2. The debug x86_64 kernel ELF is 11 MB (13 MB disk image); the release one is 1.7 MB
   (3.5 MB image). Booting the **release kernel cuts x86_64 boot 15.1 s → 3.9 s (−74 %)** —
   measured via `ROSX_KERNEL_ELF` alone, no code change.
3. x86_32 boots in **0.8 s** because its `-kernel` multiboot path loads the ELF host-side —
   no emulated BIOS, no emulated disk. Same kernel family, ~20× boot asymmetry from the
   boot-medium mechanism alone.
4. Host noise is real: one dev-profile sample hit 46 s (3× the floor) on an otherwise
   quiet host; release-kernel samples were tight (3.80–3.88 s). Timing *gates* must stay
   context-gated; suite assertions must keep event-presence semantics with generous caps.

## Suite baselines (dev profile, as CI runs today)

### x86_64 — wall 11 m 35 s (test sum 686 s)

| Test | Time | Test | Time |
|---|---|---|---|
| `bench_allocator` | 71.3 s | `stress_mixed_preemption` | 16.6 s |
| `bench_alloc` | 68.1 s | `ipc_wake_latency` | 16.5 s |
| `app_lifecycle` #1 | 55.8 s | `ata_stress` | 16.4 s |
| `app_lifecycle` #2 (`tetris_survives_idle`) | 62.4 s | `shell_command_arguments` | 16.3 s |
| `ipc_bulk` | 57.8 s | `ata_read` | 15.9 s |
| `vfs_hdd_flow` | 51.8 s | `kstats_mem` | 15.9 s |
| `ipc_random` | 22.8 s | `elf_arguments` | 15.8 s |
| `ata_env_spike` | 20.3 s | `waitset_or` | 15.8 s |
| `ipc_vfs` | 16.7 s | `ipc_ping` | 15.5 s |
| `ata_write` | 16.6 s | `boot_and_shell` ×4 | 15.1–15.5 s |
| | | `ata_identify` | 15.3 s |
| | | `iso_boot` / `bench_baseline` | 0 s (skip / ignored) |

`app_lifecycle` #1 includes a 21.6 s on-demand dev kernel build (the harness wants the
dev profile; a preceding `cargo xtask build` leaves release). Twenty per-binary kernel
build invocations: 0.15–0.18 s each warm, one cold.

### x86_32 — wall 5 m 11 s (test sum ~311 s)

| Test | Time | Test | Time |
|---|---|---|---|
| `app_lifecycle` #2 | 61.3 s | `stress_mixed_preemption` | 2.8 s |
| `bench_alloc` | 51.2 s | `iso_boot` | 2.8 s |
| `bench_allocator` | 50.3 s | `ipc_wake_latency` | 2.1 s |
| `app_lifecycle` #1 | 47.8 s | `ata_write` / `ata_stress` | 1.8 s |
| `ipc_bulk` | 40.8 s | `shell_command_arguments` | 1.7 s |
| `vfs_hdd_flow` | 23.6 s | `waitset_or` | 1.6 s |
| `ipc_random` | 9.9 s | `ata_identify` / `ata_read` | 1.4–1.5 s |
| | | rest (`ipc_ping`, `kstats_mem`, boot…) | 0.6–1.4 s |

### Cost model

- **x86_64 is boot-dominated:** 24 boots × ~15.2 s ≈ 364 s ≈ 53 % of test time.
- **x86_32 is work-dominated:** boots total ~19 s ≈ 6 %; the tail is guest work
  (benches 101 s, `ipc_bulk` 41 s, `vfs_hdd_flow` 24 s) plus `app_lifecycle`'s
  **host-side fixed sleeps** (~90 s of its 109 s are `thread::sleep` waiting windows).
- The six long tests (`app_lifecycle`×2, `bench_alloc`, `bench_allocator`, `ipc_bulk`,
  `vfs_hdd_flow`) are ~56 % of x86_64 and ~74 % of x86_32 test time.

## Release-kernel suite runs (`ROSX_KERNEL_ELF` → release ELFs, same serial protocol)

| Arch | dev wall | release wall | Δ |
|---|---|---|---|
| x86_64 | 11 m 35 s | **6 m 0 s** | **−48 %** |
| x86_32 | 5 m 11 s | 4 m 49 s¹ | −7 % |

¹ includes a one-off 61 s timeout failure in `vfs_hdd_flow` (see flake note); a healthy
release run is ≈ 4 m 7 s.

x86_64 release per-test shape: simple-test floor 3.9–4.6 s; tail `bench_alloc` 55.8,
`bench_allocator` 54.4, `app_lifecycle` 50.7 + 41.0, `ipc_bulk` 45.9, `vfs_hdd_flow` 26.4.
x86_32 release floor stays ~0.6 s; same six-test tail.

**Flake observed (x86_32 release, during baseline):** `vfs_hdd_flow` failed once — after
boot reached the prompt, the session produced *zero* further output (no keystroke echo)
until the 60 s timeout; an immediate re-run passed in 19.6 s. Zero echo means the typed
keystrokes never arrived, implicating the fire-and-forget `sendkey` path under host noise
(no echo-ack, fixed 50 ms pacing) rather than the release kernel itself
(`boot_and_shell`, which also types, passed the same run).

## Optimization mapping (data-driven)

| Lever | Measured / estimated saving | Evidence |
|---|---|---|
| **CI: run tests on the release kernel via `ROSX_KERNEL_ELF`** | **measured: x86_64 suite 11m35 → 6m0s (−48 %)**; boot 15.1 → 3.9 s; skips 20 build invocations + 21.6 s cold rebuild | measured |
| Tier: benches/stress out of PR runs | −139 s x86_64 (−20 %), −101 s x86_32 (−33 %) | `bench_alloc`, `bench_allocator` (precedent: `bench_baseline` `#[ignore]`) |
| Scenario grouping (24 boots → ~6) | −~270 s on dev x86_64; −~19 s x86_32 (boots already cheap there) | boots dominate only x86_64 dev |
| Parallel runner (4-way) | up to ÷3–4 wall once per-session boot images exist | blocker: x86_64 boot image at fixed shared path |
| Trim `app_lifecycle` host-side sleeps | up to −~60 s both arches | fixed idle-sleep windows, not guest work |
| KVM auto-detect (local dev, x86_64) | boot → sub-second expected | `/dev/kvm` available on dev boxes; CI (no nested virt) unaffected |
| Boot-medium work for x86_64 (smaller image / leaner pre-kernel path) | 3.9 s floor is still 5× x86_32's 0.8 s | pre-kernel SeaBIOS+stage2 path is the remaining cost |

Composition estimate, stacking on the measured release-kernel run (x86_64, 360 s test
sum): −~110 s bench tiering −~60 s `app_lifecycle` sleep trim ≈ ~190 s, then scenario
grouping and/or 4-way parallel ≈ **under a minute of test time**, vs 695 s today.

## Phase 1 landed (measured on this host, same protocol)

| | dev, serial (baseline) | release kernel + PR tier (now in CI) | Δ |
|---|---|---|---|
| x86_64 | 11 m 35 s | **4 m 02 s** | **−65 %** |
| x86_32 | 5 m 11 s | **2 m 37 s** | **−49 %** |

Implemented by: the boot-measurement test (`8f65004`), the new x86_64 CI integration job
plus release-kernel booting of both suites (`0e21eed`), and ignoring the bench/stress
tests outside explicit `--ignored` runs (`4f5c774`). The two CI jobs run in parallel, so
PR integration wall time is ~4 min on a runner class close to this host. Next levers
from the table above remain: parallel runner (biggest structural win, needs per-session
boot images), `app_lifecycle` sleep trim, echo-ack sendkey pacing, scenario grouping.

## Captured decisions

1. **Add an x86_64 integration job to CI** (today only x86_32 runs the suite).
2. Isolation remains the ideal; the goal is isolation *without* a full boot per test.

## Follow-up experiments proposed (not yet approved/run)

- `system_reset`-based session reuse: one QEMU process, reset between tests, TCG code cache
  warm — measures the realistic "isolation without boot" floor. Needs a session variant
  without `-no-reboot` (small harness change). Less urgent on the release kernel
  (3.9 s boot); more interesting if full fresh-process isolation is kept at high
  parallelism where boot still dominates test work.
- Echo-ack sendkey pacing (send char, wait for echo, cap the wait): removes the flake
  window observed in this baseline and removes dead 50 ms per keystroke.
- Fine-grained x86_64 boot attribution (SeaBIOS vs stage2 vs jump to kernel) to see if the
  remaining 3.9 s has a removable component.
- Nightly-tier workflow for benches/stress + the x86_64 CI job definition.
