# Integration-suite speed-up — NIC-less boot tax, then shared sessions

> **Date:** 2026-11-22 · **Branch:** `ccr-297312b4-26oajz` · **Base:** `origin/main@c1a1d8a`
> **Status:** done on branch `integration-speedup` (2026-11-22). Steps 0–6 landed; the measured
> result is in `plans/integration-baseline-2026-11-22.md` — suite wall 907 s → 246 s (x86_64) and
> 707 s → 149 s (x86_32) at the merge base, 155 s / 148 s after merging main's x86_64 carrier swap;
> 60 QEMU boots → 37 per arch; boot-to-prompt median 14.69 s → 4.31 s and 11.64 s → 1.68 s, with
> x86_64 at 1.72 s once the carrier swap landed; 0 flakes in the measured runs.
> **Evidence base:** `plans/integration-baseline-2026-11-07.md` (boot-to-prompt
> distribution, cost model) and `plans/integration-flakiness-assessment-2026-11-20.md`
> (mechanisms M1–M9, per-test value). Nothing here was measured on the authoring
> host (no QEMU); Step 0 produces the reference numbers.

## Goal

Cut the wall time of `cargo test -p tests-integration` on both arches without
losing coverage or adding flakes. About 65 QEMU boots per arch run today, each
at a ~15 s floor, and the suite is boot-dominated on x86_64.

## Decisions taken

1. The `ne2kd` probe tax is fixed first, in `init`, before any harness work.
2. A failed reset between tests in a shared session is a hard failure (simplest
   policy); revisit only if it proves noisy.
3. One merged test binary per shared group; no long-lived harness daemon.
4. x86_64 drives the design; x86_32 is checked for no regression.
5. The baseline is captured and committed before any change.
6. The `init` fix waits with a waitset and uses the inter-probe sleep as its
   bounded wait; no new syscall (a new syscall would be an ABI change and needs
   approval first).

## Where the time goes

- **The probe tax (M8).** `apps/init/src/main.rs` `start_service` discards the
  future returned by `Syscall::load_with_args`, then `await_service` polls the
  IPC probe 100 × 100 ms. `ne2kd` exits at once when no NIC is present, yet init
  still waits the full 10 s. Nearly every session has no NIC.
- **One boot per test.** `QemuSession` spawns a fresh QEMU per test function;
  about 31 boots belong to tests that only type shell commands into a fresh
  shell (`shell_background` 5, `exit_status` 5, `ted_edit` 6, `ctrl_c` 4,
  `ps_listing` 4, `boot_and_shell` 4, `kstats_mem` 3).
- **Cargo fail-fast** hides the remaining binaries after the first flake.

## Step 0 — capture and commit the baseline

Needs a QEMU host; the authoring container has none.

- Preconditions: `uptime` load against `nproc`, and
  `pgrep -f 'qemu-system|cargo test'` empty (Nov-21 erratum of the flakiness
  assessment).
- Per arch:
  `ROSX_KERNEL_ELF=<release kernel> [ROSX_ARCH=x86_32] cargo test -p tests-integration -- --test-threads=1 --no-fail-fast -Zunstable-options --report-time`
- Also run `measure_boot` (`-- --ignored --nocapture`) for the per-phase boot
  split, so the probe tax shows as its own number.
- Record per-test time, boot count, total wall and every flake with its
  signature in `plans/integration-baseline-2026-11-22.md`, reusing the table
  layout of the 2026-11-07 baseline so the two compare directly.
- Commit: the baseline doc only.

## Step 1 — stop probing a service that already died

- `start_service` keeps the `UserFutureHandle` from `load_with_args`.
- `await_service` attaches that completion future to a waitset and, between
  probe attempts, waits on it with the existing 100 ms sleep as the bound. If
  the completion fires, it stops probing and prints a distinct line, for
  example `[INIT] <probe> service exited before ready`, instead of burning the
  remaining attempts and printing `not ready`.
- The waitset is dropped on every exit path of the function.
- Open item to resolve while implementing: confirm the waitset API can be
  combined with a sleep this way. If it cannot, stop and ask before touching
  `system` or the syscall ABI.
- Tests: unit tests where the logic is pure; an integration assertion that a
  NIC-less boot reaches `rose>` and logs the new line, with a loose time bound
  only (timing gates must stay generous); `ne2k_smoke` must still pass, proving
  a present NIC is still probed to `ready`.
- Gate: `cargo xtask check`, then one `cargo xtask verify` (touches boot on
  both arches).
- Measurement: re-run the Step 0 protocol and append the numbers to the
  baseline doc in the same commit or the next.

## Step 2 — harness primitives for shared sessions

Harness-only, in `tests-integration/src`:

- A `SharedSession` wrapping `QemuSession` with `reset()`: Ctrl-C, wait for a
  fresh `rose>` after a new mark, run `ps`, assert only the baseline process
  set is alive.
- Reset failure panics with the transcript and says it was a reset failure, so
  a dirty VM is never reported as the next test's failure.
- `expect_disk_boot()` from `docs/todo/harness-disk-boot-mount-proof.md`
  (waits for `mount ok device=ata0d0p0 dir=/`); delete that todo in the same
  commit.
- Unit tests for any pure logic (process-set comparison, mark handling).

## Step 3 — pilot one group

- Merge `exit_status` (5 tests) and `boot_and_shell` (4 tests) into one binary
  with one session, run in a fixed order with `reset()` between tests.
- Compare against the baseline for time and flakiness. Stop here and review
  before going wider; this is where the design assumption is tested.

## Step 4 — remaining shell-only groups

- Same pattern for `ctrl_c`, `shell_background`, `ps_listing`,
  `kstats_mem`'s snapshot test, `elf_arguments`, `shell_command_arguments`,
  `future_ownership`, `waitset_or`.
- Keep their own VM: `kstats_mem`'s repeated-run memory test (measures absolute
  used bytes), `ted_edit` (writes to disk), and everything with a special disk
  topology, reboot or network (`*_hdd_flow`, `fat_partition_flow`,
  `expose_corrupt_hdd`, `blockd_refresh_witness`, `ata_*`, `disk_boot`, `ne2k_*`,
  `arp_*`, `ip_*`).
- Expected: boots drop from ~31 to ~4–6 per arch for these groups.

## Step 5 — CI

- Add `--no-fail-fast` to both integration jobs in `.github/workflows/ci.yml`
  so one flake no longer hides the other binaries (M7). The job stays red on
  failure. Keep `--test-threads=1`; parallel QEMU is deliberately not part of
  this plan (contention turns Med flakes near-certain).

## Step 6 — final measurement and docs

- Re-run the Step 0 protocol on both arches; append before/after to the
  baseline doc.
- Update `docs/architecture/boot-media-and-harness.md` and
  `tests-integration/README.md` for the shared-session model.

## Risks

- **Dirty shared state** (line buffer, background jobs, PIDs, memory):
  mitigated by `reset()` and the hard-fail policy; tests already assert on
  output since a mark, which limits cross-test leakage.
- **Failure attribution:** a wedged VM fails the rest of its group;
  the reset-failure message must name the group and the last test.
- **Ordering:** tests in a group run in a fixed order and must not depend on
  each other's output.
- **Flake background:** M2 (keystroke echo) and M9 (disk vanishes under load)
  are unaffected by this plan; fewer boots reduces M9's exposure.

## Out of scope

Parallel QEMU, KVM auto-detection, `system_reset`-based session reuse, a
long-lived harness daemon, and fixes for M1–M5/M9. Each is a follow-up once the
Step 6 numbers exist.

## Process

Each step ends with `cargo xtask check` green, a commit with no message prefix,
and a push to the working branch, then the `retro` skill's step-end mode.
Steps 1 and 3 also get a full `cargo xtask verify` on a quiet host.
