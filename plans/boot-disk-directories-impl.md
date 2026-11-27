# Boot Disk Directories — implementation evidence (redo)

> Executed 2026-11-24 on branch `boot-disk-directories`, base `325cf5f` = `origin/main`, per
> `plans/boot-disk-directories-plan.md`. Ground-up redo of PR #184 (`organize-app-directories`,
> base `0ab4ce5`), which is left open for reference. Owner corrections carried into this run:
> no `boot/` directory (the boot chain pins `KERNEL.ELF`/`INITRD.BIN` at the volume root),
> disk `init` to `/bin`, scope stays PR #184's.

## Commits (one per plan step, gate green at every step)

1. `d1ffe01` **`system::boot_disk` treaty** — six directory constants (`bin games tests bench
   stress srv`), `Directory::name()/path()`, the 43-app table (`init` → `bin`), `directory_of`,
   `apps_in` (later review: no non-test caller, moved into `#[cfg(test)]` as a private helper),
   `SEARCH_PATH` (`/bin → /tests → /games → /bench → /stress`); 13 unit tests pin
   membership, uniqueness, 8.3 names, path order and `srv` exclusion. `cargo test -p system`:
   116 passed.
2. `473221c` **xtask staging** — `fill_volume` mmds the six directories and mcopy-stages each app
   into `::/<dir>/<name>` through the table (a `USER_APPS` name missing from the treaty fails the
   build by name); `KERNEL.ELF`/`INITRD.BIN` staging unchanged at the root;
   `require_names_fit_8_3` extended to directory names; the `create_bootable_disk` assertion now
   requires all six directories in the `mdir` listing. Verified on the real artifact: `mdir` of
   `target/rosx-boot-disk.img` shows root = six directories + `KERNEL.ELF` + `INITRD.BIN`, with
   bin 18, games 3, tests 7, bench 2, stress 2, srv 11 (43 ELFs).
3. `1c537c1` **shell search path** — `launcher.rs` `resolve()`: leading `/` used as-is, otherwise
   the `SEARCH_PATH` stat walk, first hit wins; a full miss still prints `Unknown command: <name>`
   (the harness retry logic keys on that string); a directory hit still answers `Not an
   executable`. 11 shell command tests pass.
4. `b999d08` **boot plans (both arches)** — `/randomd|/echo|/pong|/pcid|/ne2kd` → `/srv/…`,
   `exec /shell` → `exec /bin/shell`, `exec rom:shell` fallback unchanged. Plan blob grows ~29
   bytes to ~331 of the 512-byte budget (builder-asserted).
5. `6e10fe8` **app path consumers** — the `vfs` self-test stats `/srv/randomd` (the `vfs ok: stat
   randomd present` string kept, as `ipc_vfs` asserts it); `fsbench` reads `/games/snake`; the
   `ip`, `ne2k` and `arp` auto-launchers resolve daemon paths through
   `system::boot_disk::directory_of` (the three dependency arrows joined the rules file,
   `Cargo.lock` refreshed).
6. `96346cf` **harness master through the treaty** — `fat_disk::copy_built_els` mmds and
   mcopy-stages each `system::boot_disk` directory instead of globbing the release dir flat; the
   now-dead `built_els()` deleted; `disk_boot.rs` stages `::/bin/shell`, `::/bin/mkdir`,
   `::/srv/randomd` and asserts the `[INIT]` lines verbatim (`starting /srv/randomd`,
   `/srv/echo not available`, `handing over to /bin/shell`, `hand-off /bin/shell unavailable`);
   the built-disk presence assertion moved to `ls /games`; `boot_and_exit_status.rs` expects
   `games` (not `tetris`) in the root listing; `elf_launch_failure.rs` types the absolute
   `/notes.txt` and `/corrupt.elf` (the root is off the search path; the reports now echo the
   typed path); the four payload-copy suites move to `/stress/pi` and `/srv/echo`;
   `xtask/arch-rules.txt` allows `tests-integration → system` and Cargo.toml carries the
   dependency.
7. `8e3199d` **docs** — `boot-media-and-harness.md` (both FAT recipes now describe the taxonomy
   tree beside the root-pinned boot files), `boot-plan.md` (`/srv/…` service tokens, `/bin/shell`
   hand-off), `shell.md` (search-path resolution invariant and detail).

## What the redo changed versus the 2026-11-19 run

- **No `boot/` directory; `init` to `/bin`.** The carrier swap (#197) pinned `/KERNEL.ELF` and
  `/INITRD.BIN` at the volume root (stage2 looks them up by fixed root path) and #201 moved
  `init`'s real source to the ROM initrd, so the old decision's premise was gone; owner chose
  six directories with `init` in `bin/` (2026-11-24).
- **All three 2026-11-19 friction items were plan steps here:** the `arch-rules` arrows (ip,
  ne2k, arp, tests-integration) are in the plan text; the `vfs`/`fsbench` absolute-path
  consumers and the `ip`/`ne2k`/`arp` `format!`-built daemon launches are step 5; the payload
  copies are step 6. None was discovered mid-run.
- **New consumers found while re-verifying (absent in the old base):**
  `boot_and_exit_status.rs` (shared-session successor of `boot_and_shell.rs`) expected `tetris`
  in the root `ls`, and `elf_launch_failure.rs` launched root-seeded `notes.txt`/`corrupt.elf`
  by bare name — both migrated in step 6.
- **Harness `fat_disk.rs` and `xtask boot_disk.rs` were rewritten** (chain-based staging) since
  the old branch; steps 2 and 6 were written against the new code, not rebased patches.

## Verification

- `cargo xtask check`: all stages green after every code commit (pre-commit hook) and on the
  final head.
- `cargo test -p system`: 116 passed (13 new `boot_disk` taxonomy pins).
- `cargo xtask build` both arches: `mdir` audit of `target/rosx-boot-disk.img` shows exactly the
  six-directory tree with the pinned membership (bin 18, games 3, tests 7, bench 2, stress 2,
  srv 11) plus `KERNEL.ELF` and `INITRD.BIN` at the root; `target/rosx-i486-boot-disk.img`
  written by the same recipe.
- x86_32 `disk_boot` suite (`ROSX_ARCH=x86_32 ROSX_KERNEL_ELF=…/rosx-i486/release/rosx-x86_32
  cargo test -p tests-integration --test disk_boot -- --test-threads=1`): **4 passed** — staged
  master boots `/srv/randomd`, reports `/srv/echo not available`, hands over to `/bin/shell`,
  root mount lists directories, `ls /boot` reads nested FAT, root stays writable, the built
  image serves `snake`/`tetris` through `ls /games`, a disk without `/bin/shell` falls back to
  `rom:shell`, and an initrd-less disk panics naming `INITRD.BIN`.
- x86_64 migrated-suite set (`ROSX_ARCH=x86_64 ROSX_KERNEL_ELF=…/rosx/release/rosx`,
  `boot_and_exit_status ctrl_c elf_launch_failure expose_corrupt_hdd vfs_hdd_flow
  fat_partition_flow partitioned_hdd_flow ipc_vfs ps_listing shell_background
  shell_single_shots`): **12 passed, 0 failed** — bare-name resolution through the search path
  (`pi`, `bulk`, `ping`, `bench`, `ps`, `exiter`, `spinner`, `mem`, `ata`, `mount`, `cp`…), the
  `games` root listing, absolute `/notes.txt` and `/corrupt.elf` launches, the `vfs ok: stat
  randomd present` pin against `/srv/randomd`, and the taxonomy payload copies.

## Full-suite closure (quiet host, one suite at a time, exit codes captured)

- x86_64 `cargo xtask test`: **1985 passed, 0 failed, 6 ignored** (the benchmark tests, ignored
  by design), 192 s, boot-test 26 checkpoints 0 failures.
- x86_32 `ROSX_ARCH=x86_32 cargo xtask test`: **1985 passed, 0 failed, 6 ignored**, 195 s, boot-test
  26 checkpoints 0 failures; the `disk_boot` suite ran its four tests there (staged master through
  `/bin` and `/srv`, built `target/rosx-i486-boot-disk.img`, ROM fallback, initrd panic).

## Not verified here

Hardware. The era/smoke media are untouched and carry no apps.

## Post-review: consumers never launch daemons (2026-11-25)

Code review of `6e10fe8` on the PR found the daemon auto-launch in `ip`/`ne2k`/`arp`
to be a coupling to avoid (app CLIs reaching VFS + `system::boot_disk` to bootstrap
services, duplicating the shell launcher). Owner ruling: remove it — services are
provisioned ahead of use (boot plan for non-optional ones, manual `/srv/ipd &` for
`ipd`, whose old "no boot-plan step" rationale predated `&` job control), and a
future service-manager daemon owns on-demand start. `268e341` dropped
`launch_detached`/`ensure_service` from the three CLIs (their `no service` messages
are the failure UX), shed the `system`/`pci-protocol`/`vfs-protocol` arrows from all
three crates, and pointed `ip_slirp`/`ip_inject` at
`tests_integration::shared::start_ipd`. Decision recorded in
`docs/decisions/service-provisioning-and-consumer-launch.md`; `network-and-ip.md`
updated (its stale `/pcid`, `/ne2kd` plan-path mentions fixed in the same pass).
`ip_inject ip_slirp arp_slirp arp_watch ne2k_smoke ne2k_watch boot_and_exit_status`
on x86_64: 7 passed, 0 failed. Full `cargo xtask test` re-run at `b9d27b2` on both
arches: 1985 passed, 0 failed, 6 ignored each, boot-test 26 checkpoints 0 failures.

## Post-review: the plan keeps only non-optional services (2026-11-26)

Owner ruling following the provisioning decision: `randomd`, `echo` and `pong`
leave the boot plan too — they exist for demos and for a few IPC suites, not for
every boot. Both arches now plan `pcid` and `ne2kd` from `/srv/` and nothing
else from disk. The suites that need the daemons start them at the prompt:
`shared::start_ipd` generalized into `start_service(session, program, ready)`,
used by `ipc_random` (`/srv/randomd`), `ipc_ping` and `ipc_wake_latency`
(`/srv/pong`, wake-latency also needs `/srv/randomd`), `ipc_bulk` (`/srv/echo`);
the daemons themselves, their taxonomy placement and the disk staging are
unchanged. `disk_boot` re-anchored: the fixture disk proves the log-and-continue
policy on `/srv/pcid not available` (fixture no longer stages `randomd`), the
built disk proves a live plan service with `starting /srv/pcid` + `PCI ready`
(`ne2kd` is not asserted because the disk-boot QEMU has no NIC attached). The
shell `random` built-in now needs `/srv/randomd &` typed first; it already
degrades gracefully (`Connection failed`) without it. Docs: `boot-plan.md`
shared-plan list and `boot-media-and-harness.md` service sentence narrowed,
`shell.md` pong phrasing de-scoped from "the boot plan's daemon", decision
record extended. Verification: `ipc_random ipc_ping ipc_wake_latency ipc_bulk
boot_and_exit_status` on x86_64 — 5 passed, 0 failed; same four plus `disk_boot`
(4 tests) on x86_32 — 5 passed, 0 failed.

## PR comments round 2 (2026-11-26)

Second review pass on #204, five findings. **Must-fix 1:** `boot_disk::apps_in` was test-only
production surface (AGENTS.md bans it — the review commits had removed its last consumers);
deleted from the public API, a private helper now lives in the `#[cfg(test)]` module, the 13
pins unchanged. **Must-fix 2:** the `start_service` insertion had collapsed
`process_names`'s first two lines into one — restored, and a sweep over the whole PR diff
(`{` + indented code on one line) found no other artifact; note the repo is intentionally not
default-`rustfmt` shaped (no rustfmt.toml, no fmt CI step), so the fix is the lost newline,
not a reformat. **Should-fix:** `plans/boot-disk-directories-plan.md` amended in place —
header note plus correction marks on fact 3 (final 11 steps), fact 5 and step 6 (the
`/srv/pcid not available` + `PCI ready` assertions, the `start_service`/`start_ipd` helpers),
fact 7 and step 5 (auto-launch removed, arrows shed). **Optional:** `shell.md` now states the
resolve edge cases (single-entry lookup, no PATH semantics for names containing `/`, only
`NotFound` advances — other VFS errors abort rather than mask, unknown name costs ≤ 5 stats);
no unit pin because `resolve` takes the concrete `Vfs` client and faking it would mean a trait
abstraction out of scope here. `boot-media-and-harness.md` states that the `disk_boot` fixture
proves missing service **file** with `srv/` present, not missing directory.

## Friction log

step 2 | `cargo xtask build x86_32` was launched in the background while the step-3 launcher edit was still in flight; it compiled the half-edited tree and failed with 2 errors in `shell`, costing one rebuild | ENV-TRAP | 1 round
verification | first x86_32 `disk_boot` run set `ROSX_KERNEL_ELF=target/rosx-x86_32/release/rosx-x86_32`, but the x86_32 kernel ELF ships under `target/rosx-i486/release/rosx-x86_32` (dir is `kernel_dir_name`, not the package name); all 4 tests panicked on canonicalization, no product signal | MISSING-FACT | 1 round
verification | a bare `cargo xtask test` covers only the `ROSX_ARCH` arch (default x86_64); the x86_32 integration pass needs its own `ROSX_ARCH=x86_32 cargo xtask test` run — read from the log files, not the exit code | MISSING-FACT | 0
post-review | the redo re-verified every repo fact but carried #184's daemon auto-launch as-is; review caught the VFS + `boot_disk` coupling in three CLIs and it was removed in `268e341` — a redo must question inherited design, not only refresh facts | PLAN-DEFECT | 1 round
