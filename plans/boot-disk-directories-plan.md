# Boot Disk Directories — organizing the FAT root into bin / games / tests / bench / stress / srv / boot — Plan

> **Status:** Plan (code starts at step 1). Written 2026-11-19 on branch `organize-app-directories` (base `0ab4ce5` = `origin/main`).
> **Feature:** the FAT boot disk stops carrying all 43 user ELFs flat at the volume root and
> instead ships a curated directory tree — `bin`, `games`, `tests`, `bench`, `stress`, `srv`, and
> the existing `boot` — with the layout owned by a new `system::boot_disk` treaty module consumed
> by the xtask recipe, the shell launcher, and the integration harness, and shell name resolution
> extended to a fixed search path so bare-name launches keep working unchanged.
> **Method:** every repo fact below was re-verified against this working tree (paths cited).

**Decisions (owner-approved in session 2026-11-19):**
- Taxonomy: **bin, games, tests, bench, stress**, plus **`srv/`** for the service daemons and
  **`init` moved into the existing `boot/`** next to `kernel.elf` and `rom.img`.
- `pi` lives in **`stress/`** — it is a long-running CPU hog (used as one in `ctrl_c.rs`), not a game.
- Single source of truth: a new **`system::boot_disk`** module holding the directory names and the
  app→directory table, consumed by `xtask`, the shell and the harness so the three can never
  drift. Precedent: `system::rom` already owns the ROM image layout; media layout is the same
  "contract, not mechanism" category. (Touches `system` — approved as part of this plan.)
- The shell search path is **`/bin → /tests → /games → /bench → /stress`**. `srv/` is deliberately
  off the path: services are started by the boot plan with full paths, and a bare `pong` at the
  prompt must not spawn a second daemon that dies on bind.

---

## 1. Goal & Non-Goals

### Goal
- `cargo xtask build` on both arches writes a FAT boot disk whose root lists the seven directories
  instead of 43 files, with every user ELF in its category directory and `init` in `boot/`.
- The integration harness full-apps FAT master mirrors the same tree from the same table.
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

## 2. Final layout (both arches, xtask disk and harness master)

```
/bin     shell vfs cp rm mkdir rmdir format mount attach mem ps ted ata pci ne2k arp ip   (17)
/games   snake tetris conway                                                               (3)
/tests   bulk ping threads waitset leaky exiter spinner                                   (7)
/bench   bench fsbench                                                                    (2)
/stress  stress pi                                                                        (2)
/srv     randomd echo pong atad blockd vfsd inputd ps2d pcid ne2kd ipd                    (11)
/boot    kernel.elf rom.img init                                                          (3)
```

## 3. Verified facts (impact surface)

1. `xtask/src/boot_disk.rs::mcopy_apps` copies every `USER_APPS` ELF to `::/` in one `mcopy`;
   `mmake_boot_directory` already `mmd`s `::/boot`. `require_names_fit_8_3` checks app stems only.
2. `apps/shell/src/launcher.rs:26` resolves every command as exactly `format!("/{}", cmd.name)`;
   failure prints `Unknown command: <name>` — the harness retry logic in
   `tests-integration/src/qemu.rs::run_command` keys on that string.
3. Boot plans (`arch/x86_64/src/main.rs`, `arch/x86_32/src/main.rs`) run
   `.service("/randomd"|"/echo"|"/pong"|"/pcid"|"/ne2kd")` and `.exec("/shell")` after
   `run rom:mount ata0d0p0 /`; `exec rom:shell` is the fallback. Init treats plan tokens as
   literal VFS paths (`apps/init/src`: `Program::Vfs(path) → read_from_vfs(path)`).
4. `tests-integration/src/fat_disk.rs::create_boot_disk` globs the user release dir
   (`built_els`, names without a dot) and `mcopy`s everything flat to `::/`.
5. `tests-integration/tests/disk_boot.rs` stages `shell`/`mkdir`/`randomd` at `::/<app>` and
   asserts `[INIT] starting /randomd`, `[INIT] handing over to /shell`, `[INIT] hand-off /shell
   unavailable`, and a root `ls` containing `snake` and `tetris`.
6. Shell-launched names across the integration suite (all keep working via the search path):
   `exiter mount mkdir attach cp ne2k format bench ata rm pi arp spinner rmdir ping mem leaky
   waitset stress ps fsbench bulk` (`ls sleep random` are built-ins).
7. `cargo xtask check-isa` audits the release dir by app name, not the disk layout — unaffected.
   `apps/dummy` is a lib-only member with no ELF; it never reaches the disk.
8. The GRUB ISO / era media carry only `/boot` files — unaffected.

## 4. Implementation steps (one commit each, gate green at every step)

1. **`system/src/boot_disk.rs`** — the treaty module: seven directory-name constants, the
   app→directory table covering all 43 `USER_APPS`, and a lookup. `pub mod boot_disk;` in
   `system/src/lib.rs`.
2. **`xtask/src/boot_disk.rs`** — consume the table: `mmd` every directory (generalize
   `mmake_boot_directory`), `mcopy` each group into its directory instead of the flat `::/`;
   extend `require_names_fit_8_3` to directory names.
3. **`apps/shell/src/launcher.rs`** — name starting with `/` is used as-is; otherwise walk the
   search list from `system::boot_disk`, stat-first, first hit wins; a full miss still prints
   `Unknown command: <name>`. Update shell unit tests.
4. **Boot plans (both arches)** — `/randomd → /srv/randomd`, `/echo → /srv/echo`,
   `/pong → /srv/pong`, `/pcid → /srv/pcid`, `/ne2kd → /srv/ne2kd`, `exec /shell →
   exec /bin/shell`; `exec rom:shell` unchanged. Well inside the plan's 512-byte budget.
5. **`tests-integration/src/fat_disk.rs`** — `create_boot_disk` stages through the shared table
   (`mmd` + per-directory `mcopy`) instead of a flat root.
6. **`tests-integration/tests/disk_boot.rs`** — stage at the new paths (`::/bin/shell`,
   `::/bin/mkdir`, `::/srv/randomd`); update the `[INIT]` message assertions
   (`starting /srv/randomd`, `handing over to /bin/shell`, `hand-off /bin/shell unavailable`)
   and the `ls` assertions (root lists directories; app presence asserted through the directory).
7. **Docs** — `docs/architecture/boot-media-and-harness.md` (the "every user ELF at the root"
   sentences), `docs/architecture/shell.md` (search-path resolution rule),
   `docs/architecture/boot-plan.md` (plan example tokens).

## 5. Verification

- Steps 1–2: `cargo xtask check`; `cargo xtask build` on both arches and `mdir -i` the produced
  image to confirm the tree.
- Step 3: shell unit tests, plus a QEMU boot (`cargo xtask run`) exercising `snake`,
  `bench list`, `/games/snake`, and an unknown name (`Unknown command`).
- Steps 4–6: `ROSX_ARCH=x86_32 cargo test -p tests-integration disk_boot -- --test-threads=1`
  (the plan-message and layout suite), and the x86_64 boot to disk shell.
- End state: full `cargo xtask check` and `cargo xtask test`.

## 6. Deliberate behaviour changes

- After the root mount, a bare `ls` at the prompt lists **directories**, not apps; `ls bin`,
  `ls games` is the new browsing idiom. The `ls` builtin itself is unchanged.
- `srv/` is off the shell search path: services are started by the boot plan with full paths;
  typing `pong` answers `Unknown command`, which prevents a stray second daemon racing the
  bind-name.
