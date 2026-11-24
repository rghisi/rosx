# Boot Disk Directories — organizing the FAT root into bin / games / tests / bench / stress / srv — Plan

> **Status:** Plan (code starts at step 1). Written 2026-11-24 on branch `boot-disk-directories`
> (base `325cf5f` = `origin/main`). This is a ground-up redo of `plans/boot-disk-directories-plan.md`
> as executed on `organize-app-directories` (PR #184, base `0ab4ce5`): the codebase moved ~15 PRs
> past that point (carrier swap #197, integration rework #196/#200, ROM-as-initrd #201, ELF load
> at the syscall #191, paged arenas #202), so the old branch is not rebased — the plan is
> re-verified against today's tree and corrected. PR #184 stays open for reference.
> **Feature:** the FAT boot disk stops carrying all 43 user ELFs flat at the volume root and
> instead ships a curated directory tree — `bin`, `games`, `tests`, `bench`, `stress`, `srv` —
> with the layout owned by a new `system::boot_disk` treaty module consumed by the xtask recipe,
> the shell launcher, and the integration harness, and shell name resolution extended to a fixed
> search path so bare-name launches keep working unchanged.
> **Method:** every repo fact below was re-verified against this working tree (paths cited).

**Decisions (owner-approved in session 2026-11-19, carried over):**
- Taxonomy: **bin, games, tests, bench, stress**, plus **`srv/`** for the service daemons.
- `pi` lives in **`stress/`** — it is a long-running CPU hog (used as one in `ctrl_c.rs`), not a game.
- Single source of truth: a new **`system::boot_disk`** module holding the directory names and the
  app→directory table, consumed by `xtask`, the shell and the harness so the three can never
  drift. Precedent: `system::rom` already owns the ROM image layout; media layout is the same
  "contract, not mechanism" category. (Touches `system` — approved as part of this plan.)
- The shell search path is **`/bin → /tests → /games → /bench → /stress`**. `srv/` is deliberately
  off the path: services are started by the boot plan with full paths, and a bare `pong` at the
  prompt must not spawn a second daemon that dies on bind.

**Decisions (owner-approved in session 2026-11-24, correcting the 2026-11-19 premises):**
- **`boot/` is dropped.** The old decision moved `init` into "the existing `boot/` next to
  `kernel.elf` and `rom.img`". That directory no longer exists: the carrier swap (#197) pins the
  boot files at the volume root — stage2 looks up `/KERNEL.ELF`
  (`arch/x86-bootloader/stage2/src/elf32.rs`, `longmode.rs`) and `/INITRD.BIN`
  (`arch/x86-bootloader/stage2/src/fat.rs`) by fixed root paths — and the ROM now ships as
  `INITRD.BIN` (#201), not `rom.img`. Boot files stay exactly where the chain requires.
- **`init` on disk goes to `/bin`.** The kernel boots `init` from the ROM
  (`kernel::rom::program("init")`, both arch `main.rs`), so the disk copy is never read; it stays
  on the disk (it is a `USER_APPS` ELF and the harness glob-free staging table must cover all 43)
  in `bin/`. Accepted consequence: a bare `init` at the prompt launches it; after boot the ROM is
  released, so it prints `[INIT] no ROM mapping` and exits clean (`apps/init/src/main.rs`), and
  with an empty plan `[INIT] empty boot plan`. Harmless.
- **Scope stays PR #184's.** The duplicated `KERNEL.ELF`/`INITRD.BIN` name constants
  (`xtask/src/boot_disk.rs`, `xtask/src/boot_image.rs`, `tests-integration/src/fat_disk.rs`) are
  *not* folded into the treaty.

---

## 1. Goal & Non-Goals

### Goal
- `cargo xtask build` on both arches writes a FAT boot disk whose root lists the six directories
  plus `KERNEL.ELF` and `INITRD.BIN` instead of 43 loose ELFs, with every user ELF in its category
  directory.
- The integration harness boot master (`fat_disk::create_boot_master`) mirrors the same tree from
  the same table.
- Bare-name launches at the disk shell (`bench list`, `exiter 7`, `snake`) keep working through a
  shell search path, so the ~20 integration tests that type bare names need no edits.
- Explicit paths still work at the prompt (`/games/snake`).

### Non-Goals
- The ROM image and `BOOTSTRAP_APPS` stay exactly as they are — the ROM is not a VFS volume.
- No PATH variable or configuration mechanism: the search list is a compiled-in constant.
- No per-directory special handling in `vfsd` or `fs/fat` — nested FAT reads already work
  (`disk_boot.rs` proves `/boot/menu.txt` serves through the root mount).
- No rename of any app; 8.3 stems are already satisfied and directory names (`bin` … `stress`,
  all ≤ 8) are checked too.
- `KERNEL.ELF` and `INITRD.BIN` stay at the volume root (boot-chain fixed paths); the era/smoke
  media (`xtask/src/boot_image.rs`: `PROBE.TXT`, `KERNEL.ELF`, `INITRD.BIN` only) are untouched.

## 2. Final layout (both arches, xtask disk and harness master)

```
/bin     shell vfs cp rm mkdir rmdir format mount mem ps ted ata pci ne2k arp ip init   (18)
/games   snake tetris conway                                                            (3)
/tests   bulk ping threads waitset leaky exiter spinner                                 (7)
/bench   bench fsbench                                                                  (2)
/stress  stress pi                                                                      (2)
/srv     randomd echo pong atad blockd vfsd inputd ps2d pcid ne2kd ipd                  (11)
root     KERNEL.ELF INITRD.BIN (chain-pinned, staged outside the taxonomy)              (2)
```

## 3. Verified facts (impact surface, re-verified on `325cf5f`)

1. `xtask/src/boot_disk.rs::fill_volume` mcopy-stages every `USER_APPS` ELF (43, unchanged list)
   flat to `::/<name>` with `chain::mcopy_file`, then `KERNEL.ELF` and `INITRD.BIN` flat at the
   root; `create_bootable_disk` asserts the `mdir` listing sees `KERNEL.ELF`. There is **no**
   `boot/` directory and no `mmake_boot_directory` anymore. `chain::mmd_directory` exists
   (`arch/x86-runner/src/chain.rs`) for directory creation. `require_names_fit_8_3` checks app
   stems only.
2. `apps/shell/src/launcher.rs:26` resolves every command as exactly `format!("/{}", cmd.name)`;
   failure prints `Unknown command: <name>` — the harness retry logic in
   `tests-integration/src/qemu.rs::run_command` keys on that string. A `Directory` hit answers
   `Not an executable` (kept for the search walk too).
3. Boot plans (`arch/x86_64/src/main.rs`, `arch/x86_32/src/main.rs`) run
   `.service("/randomd"|"/echo"|"/pong"|"/pcid"|"/ne2kd")` and `.exec("/shell")` after the ROM
   bootstrap (`rom:atad blockd attach vfsd inputd ps2d`, `run rom:mount ata0d0p0 /`);
   `exec rom:shell` is the fallback. The `rom:` tokens are unaffected. Init treats plan tokens as
   literal VFS paths (`apps/init/src`: `Program::Vfs(path) → read_from_vfs(path)`), echoes the
   token verbatim (`[INIT] starting {token}`), and labels tasks by basename
   (`system/src/boot_plan.rs` `Program::name()` `rsplit_once('/')`) — so `ps` rows and exit
   reports keep the bare names. Plan budget: ≤ 15 steps (now 14), ≤ 512 bytes
   (`system/src/boot_plan.rs`); this change adds ~29 bytes, inside budget, builder-asserted.
4. `tests-integration/src/fat_disk.rs`: `create_boot_master` = `create_bootable` (chain-stamped)
   + `copy_built_els` (glob of dotless ELFs in the user release dir, one flat `mcopy`) +
   `install_initrd` (`INITRD.BIN` at root) + `KERNEL.ELF` at root. `stamp_bootable` and
   `read_file_from_image` take the volume offset from the MBR. The shared boot used by
   `qemu.rs` (`spawn_shared` path) goes through `create_boot_master`, so taxonomy staging there
   reaches every shared-session suite.
5. `tests-integration/tests/disk_boot.rs` (x86_32) has **four** tests: the staged fixture
   (`shell`/`mkdir`/`randomd` at `::/<app>`, `greeting.txt` + a `boot/menu.txt` fixture, asserts
   `[INIT] starting /randomd`, `[INIT] /echo not available`, `handing over to /shell`, root `ls`
   of `greeting.txt`, `ls /boot` for nested reads, writable root); the built-disk boot (asserts
   root `ls` contains `snake` and `tetris`); the no-disk-shell ROM fallback (`hand-off /shell
   unavailable`); and the missing-`INITRD.BIN` panic (root-pinned initrd — unaffected by the
   taxonomy; the `::/boot/menu.txt` fixture is a test's own nested dir, not the dropped taxonomy
   `boot/`).
6. Shell-launched names across the integration suite (all keep working via the search path):
   `exiter mount mkdir attach cp ne2k format bench ata rm pi arp spinner rmdir ping mem leaky
   waitset stress ps fsbench bulk ted` (`ls clear rose sleep random` are built-ins; a bare `ls`
   prints the built-in names line first — layout-independent).
7. Absolute-path **data/launch** consumers that must migrate with the layout (the 2026-11-19 run
   found these late; they are first-class steps here):
   - `apps/vfs/src/main.rs:36` self-test stats `/randomd` → `/srv/randomd`; the printed line
     `vfs ok: stat randomd present` stays (asserted by `ipc_vfs.rs`).
   - `apps/fsbench/src/reads.rs:7` reads `/snake` → `/games/snake`.
   - `apps/ip/src/main.rs:31`, `apps/ne2k/src/main.rs:75`, `apps/arp/src/main.rs:239` auto-launch
     missing daemons with `vfs.read_whole(&format!("/{program}"))` — runtime-built paths; resolve
     through `system::boot_disk::directory_of`. These three crates gain a `system` dependency
     (`xtask/arch-rules.txt` rows `ip`, `ne2k`, `arp`).
   - Payload copies of root-level ELFs used as test data: `expose_corrupt_hdd.rs`,
     `partitioned_hdd_flow.rs` (`cp /pi`, `cp /echo`), `vfs_hdd_flow.rs`, `fat_partition_flow.rs`
     (`cp /pi`, `cp /fat/...`) → `/stress/pi`, `/srv/echo` with their expected `cp ok …` strings.
8. New since PR #184 — two shared-session consumers of the flat root:
   - `boot_and_exit_status.rs::shell_ls_and_unknown_command` expects `tetris` in the bare-`ls`
     root listing → expect `games` (the root lists directories now; `clear` is the built-in line,
     `Unknown command: foo` unchanged).
   - `elf_launch_failure.rs` seeds `notes.txt` and `corrupt.elf` at the master **root** and types
     the bare names; the search path does not include `/`, so it types the absolute
     `/notes.txt`, `/corrupt.elf` instead — which also proves absolute launch works (the
     `Not an executable:` reports and the `exiter <code>` recovery keep working verbatim).
9. `cargo xtask check-isa` audits the release dir by app name (`xtask/src/main.rs`
   `check_isa_command`), not the disk layout — unaffected. `apps/dummy` is a lib-only member with
   no ELF; it never reaches the disk. `xtask/src/boot_test.rs` boots the chain with fake kernels,
   no app layout.
10. `tests-integration` has no `system` dependency today (`xtask/arch-rules.txt`); the harness
    consuming the table needs the arrow added in the same commit as the staging change. `xtask`
    and `shell` already depend on `system`.
11. The era/smoke media (`xtask/src/boot_image.rs`) and the `x86-runner` disk-name checks
    (`arch/x86-runner/src/lib.rs`) never see the app tree — unaffected.

## 4. Implementation steps (one commit each, gate green at every step)

1. **`system/src/boot_disk.rs`** — the treaty module: six directory-name constants, the
   app→directory table covering all 43 `USER_APPS` (`init` → `bin`), `directory_of` lookup, and
   the shell `SEARCH_PATH`. `pub mod boot_disk;` in `system/src/lib.rs`. Unit tests pin the full
   taxonomy (every app exactly once, totals per directory, every name ≤ 8 chars, search-path
   order, `srv` off the path).
2. **`xtask/src/boot_disk.rs`** — consume the table: `chain::mmd_directory` for the six
   directories, `mcopy` each app into `::/<dir>/<name>`; `KERNEL.ELF`/`INITRD.BIN` staging
   unchanged; extend `require_names_fit_8_3` to directory names.
3. **`apps/shell/src/launcher.rs`** — name starting with `/` is used as-is; otherwise walk the
   search list from `system::boot_disk`, stat-first, first hit wins; a full miss still prints
   `Unknown command: <name>`. Update shell unit tests.
4. **Boot plans (both arches)** — `/randomd → /srv/randomd`, `/echo → /srv/echo`,
   `/pong → /srv/pong`, `/pcid → /srv/pcid`, `/ne2kd → /srv/ne2kd`, `exec /shell →
   exec /bin/shell`; `exec rom:shell` unchanged. Builder asserts the 512-byte budget at encode.
5. **App path consumers** — `apps/vfs` self-test `/srv/randomd` (print string kept); `fsbench`
   `/games/snake`; `ip`/`ne2k`/`arp` daemon auto-launch through `directory_of` with the `system`
   dependency and the three `arch-rules.txt` rows amended.
6. **Harness through the treaty** — `fat_disk::create_boot_master` stages via the table (`mmd` +
   per-app `mcopy`) replacing the flat `copy_built_els`/`built_els` glob; `arch-rules.txt` allows
   `tests-integration → system`; `disk_boot.rs` stages `::/bin/shell`, `::/bin/mkdir`,
   `::/srv/randomd` and asserts `[INIT] starting /srv/randomd`, `/srv/echo not available`,
   `handing over to /bin/shell`, `hand-off /bin/shell unavailable`; the built-disk presence
   assertion moves to `ls /games`; `boot_and_exit_status.rs` expects `games` in the root listing;
   `elf_launch_failure.rs` types `/notes.txt` and `/corrupt.elf`; the four payload-copy suites
   move to `/stress/pi`, `/srv/echo`.
7. **Docs** — `docs/architecture/boot-media-and-harness.md` (the "every user ELF at the root" and
   "all ELFs found in the user release dir" sentences, boot-plan service tokens, the root-pinned
   boot files beside the taxonomy), `docs/architecture/shell.md` (search-path resolution rule),
   `docs/architecture/boot-plan.md` (`/srv/…` service tokens, `/bin/shell` hand-off).

## 5. Verification

- Steps 1–2: `cargo xtask check`; `cargo test -p system`; `cargo xtask build` on both arches and
  `mdir -i` the produced images to confirm the tree (root = six directories + `KERNEL.ELF` +
  `INITRD.BIN`).
- Step 3: shell unit tests, plus a QEMU boot (`cargo xtask run`) exercising `snake`, `bench list`,
  `/games/snake`, and an unknown name (`Unknown command`).
- Steps 4–6: `ROSX_ARCH=x86_32 ROSX_KERNEL_ELF=… cargo test -p tests-integration --test disk_boot
  -- --test-threads=1` (captured to a file — the plan-message and layout suite), and
  `elf_launch_failure` + `boot_and_exit_status` on x86_64 (shared sessions on the taxonomy
  master).
- End state: full `cargo xtask check`, then `cargo xtask test` per arch on a quiet host (one at a
  time, output captured with exit codes per AGENTS.md).

## 6. Deliberate behaviour changes

- After the root mount, a bare `ls` at the prompt lists **directories** (plus `KERNEL.ELF`,
  `INITRD.BIN` and the built-in names line), not apps; `ls bin`, `ls games` is the new browsing
  idiom. The `ls` builtin itself is unchanged.
- `srv/` is off the shell search path: services are started by the boot plan with full paths;
  typing `pong` answers `Unknown command`, which prevents a stray second daemon racing the
  bind-name.
- `init` lives in `/bin`, so a bare `init` at the prompt launches the disk init; it finds no ROM
  mapping (released at boot) or an empty plan and exits clean.
- Bare names no longer resolve against the volume root (`/` is off the search path): files seeded
  at the root are launched by absolute path — `elf_launch_failure.rs` is the pin for that.
