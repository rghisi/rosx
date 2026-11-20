# Boot Disk Directories — implementation evidence log

Branch: `organize-app-directories`. Parent document: `plans/boot-disk-directories-plan.md`.
Running log of every milestone gate: what was run, what it printed, what it
proved. Host machine: QEMU 10.2.1, rustc 1.101.0-nightly `0abfedbc7` 2026-10-02.

## Commits and gates (in order)

1. `70e2803` plan document (`plans/boot-disk-directories-plan.md`).
2. `4aa6449` **`system::boot_disk` treaty** — `Directory` enum (Bin, Games, Tests, Bench,
   Stress, Srv, Boot), `ALL_DIRECTORIES`, `SEARCH_PATH` (bin → tests → games → bench →
   stress; srv and boot excluded), `APP_DIRECTORIES` (43 entries), `directory_of`,
   `apps_in`. 12 new unit tests pin the full taxonomy per directory, name uniqueness,
   the 8.3 stem floor on directory names, and the search-path order.
   `cargo test -p system`: 114 passed. Full `cargo xtask check`: all stages green.
3. `86bee28` **xtask recipe through the treaty** — `USER_APPS` deleted from
   `xtask/src/arch.rs` and derived from `APP_DIRECTORIES` via `user_apps()`;
   `make_directories` mmds all seven directories; `mcopy_app_directories` copies each
   group into `::/<dir>/`; `require_names_fit_8_3` extended to directory names.
   Verified on the real artifact: `cargo xtask build` + `mdir` shows
   `bin` 17 files, `games` 3, `tests` 7, `bench` 2, `stress` 2, `srv` 11,
   `boot` = init + kernel.elf + rom.img (43 ELFs + 2 boot files, root lists only the
   seven directories). Full gate green.
4. `22485a0` **shell search path** — `launcher.rs` `resolve()`: leading `/` used as-is,
   otherwise the `SEARCH_PATH` stat walk, first hit wins; a full miss still prints
   `Unknown command: <name>` (the harness retry logic keys on that string); a directory
   hit still answers `Not an executable`. Host check: lib green, 11 command tests pass
   (the bin-target host `#[panic_handler]` failure predates this branch — reproduced on
   the stashed tree). Full gate green.
5. `fb4a893` **boot plans (both arches)** — `/randomd|/echo|/pong|/pcid|/ne2kd` →
   `/srv/…`, `exec /shell` → `exec /bin/shell`, `exec rom:shell` fallback unchanged.
   Also migrated the one literal path app left behind by the grep sweep: the `vfs`
   self-test now probes `/srv/randomd` (output string `vfs ok: stat randomd present`
   kept, as `ipc_vfs` asserts it). Full gate green.
6. `2bfb971` **harness master through the treaty** — `fat_disk::create_boot_disk` mmds
   and mcopy-stages each `system::boot_disk` directory instead of globbing the release
   dir flat; the now-dead `built_els()` deleted; `disk_boot.rs` stages
   `::/bin/shell`, `::/bin/mkdir`, `::/srv/randomd` and asserts the `[INIT]` lines
   verbatim (`starting /srv/randomd`, `/srv/echo not available`,
   `handing over to /bin/shell`, `hand-off /bin/shell unavailable`); the built-disk
   presence assertion moved to `ls /games`; `boot_and_shell.rs` expects `games` (not
   `tetris`) in the root listing; `xtask/arch-rules.txt` allows
   `tests-integration → system` and Cargo.toml carries the dependency. Full gate green.
7. `7ccab3e` **docs** — `boot-media-and-harness.md` (both FAT recipes and the harness
   master now describe the taxonomy tree), `boot-plan.md` (`/srv/…` service tokens,
   `/bin/shell` hand-off), `shell.md` (search-path resolution invariant and detail).

## Where reality differed from the plan

- **The `tests-integration → system` arrow was not pre-authorized in the rule file.**
  The first full gate failed `check-arch` on it; the plan had already decided the
  harness consumes the shared table, so the rule was amended in the same commit as the
  harness staging rather than duplicating the taxonomy.
- **Two consumers outside the plan's impact list referenced the flat layout:** the
  `vfs` self-test stat'd `/randomd`, and `boot_and_shell.rs` expected `tetris` in the
  root `ls`. Both migrated alongside steps 5 and 6.
- **`Program::name()` basename rule saved the naming surface:** init labels
  VFS-launched tasks with the last path component, so `ps` rows, daemon names and the
  shell's `exiter exited with code N` messages are byte-identical to before; only the
  `[INIT]` token echo changed (asserted as planned).
- `cargo xtask check` runs the test stage with integration **excluded**
  (`run_tests(true)`), so the QEMU proof below was run explicitly.

## Verification

- `cargo xtask check`: all stages green after every code commit and once more on the
  final head (`7ccab3e`).
- `cargo test -p system`: 114 passed (12 of them the new `boot_disk` taxonomy pins).
- `cargo xtask build` both arches: `mdir` audit of `target/rosx-boot-disk.img` shows
  exactly the seven-directory tree with the pinned membership (see commit 3 above);
  `target/rosx-i486-boot-disk.img` written by the same recipe.
- x86_32 `disk_boot` suite (`ROSX_ARCH=x86_32 ROSX_KERNEL_ELF=…/rosx-x86_32 cargo test
  -p tests-integration --test disk_boot -- --test-threads=1`): **3 passed** — staged
  master boots `/srv/randomd`, reports `/srv/echo not available`, hands over to
  `/bin/shell`, root mount lists directories, `ls /boot` reads nested FAT, root stays
  writable, the built image serves `snake`/`tetris` through `ls /games`, and a disk
  without `/bin/shell` falls back to `rom:shell`.
- x86_64 smoke set (`ROSX_KERNEL_ELF=…/rosx cargo test -p tests-integration --test
  boot_and_shell --test ctrl_c --test elf_arguments --test ipc_bulk --test ipc_ping
  --test shell_background --test bench_alloc --test ps_listing -- --test-threads=1`):
  **20 passed, 1 ignored** (the bench workload, ignored by design) — proves bare-name
  resolution through the search path (`spinner`, `exiter`, `pi`, `bulk`, `ping`,
  `bench`, `ps`), the `games` root listing, and `Unknown command: foo`.

## Not verified here

The full integration suite on both arches (CI runs it); the GRUB ISO / era media are
untouched by this branch and carry no apps.

## Friction log

step 6 | `cargo xtask check` failed `check-arch`: the plan authorized the harness to consume `system::boot_disk` but never listed the `xtask/arch-rules.txt` arrow amendment it requires | PLAN-DEFECT | 1 round
step 5 | the `vfs` self-test stat'd `/randomd` and `boot_and_shell.rs` expected `tetris` in the root listing: the plan's impact-surface list missed both, caught by the pre-commit grep sweep | MISSING-FACT | 1 round
verification | `cargo test -p tests-integration disk_boot` filters test-name substrings so 2 of 3 tests were silently skipped, and `| tail -15` cut the target's result lines; re-ran as `--test disk_boot` with full capture | ENV-TRAP | 1 round
verification | believed `cargo xtask check` covered the QEMU suites; its tests stage runs with integration excluded (`run_tests(true)`) | MISSING-FACT | 0
