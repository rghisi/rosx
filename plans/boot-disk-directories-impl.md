# Boot Disk Directories — implementation evidence (redo)

> Executed 2026-11-24 on branch `boot-disk-directories`, base `325cf5f` = `origin/main`, per
> `plans/boot-disk-directories-plan.md`. Ground-up redo of PR #184 (`organize-app-directories`,
> base `0ab4ce5`), which is left open for reference. Owner corrections carried into this run:
> no `boot/` directory (the boot chain pins `KERNEL.ELF`/`INITRD.BIN` at the volume root),
> disk `init` to `/bin`, scope stays PR #184's.

## Commits (one per plan step, gate green at every step)

1. `d1ffe01` **`system::boot_disk` treaty** — six directory constants (`bin games tests bench
   stress srv`), `Directory::name()/path()`, the 43-app table (`init` → `bin`), `directory_of`,
   `apps_in`, `SEARCH_PATH` (`/bin → /tests → /games → /bench → /stress`); 13 unit tests pin
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

## Friction log

step 2 | `cargo xtask build x86_32` was launched in the background while the step-3 launcher edit was still in flight; it compiled the half-edited tree and failed with 2 errors in `shell`, costing one rebuild | ENV-TRAP | 1 round
verification | first x86_32 `disk_boot` run set `ROSX_KERNEL_ELF=target/rosx-x86_32/release/rosx-x86_32`, but the x86_32 kernel ELF ships under `target/rosx-i486/release/rosx-x86_32` (dir is `kernel_dir_name`, not the package name); all 4 tests panicked on canonicalization, no product signal | MISSING-FACT | 1 round
verification | a bare `cargo xtask test` covers only the `ROSX_ARCH` arch (default x86_64); the x86_32 integration pass needs its own `ROSX_ARCH=x86_32 cargo xtask test` run — read from the log files, not the exit code | MISSING-FACT | 0
