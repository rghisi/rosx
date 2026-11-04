# VFS: Writable ROM and an Organized Root

> **Status:** Plan (approved decisions below; no code written)
> **Decided 2026-11-04:** ROM apps move into organization directories; shell launches by relative path (`rose> games/snake`); utility tools stay at `/`; ROM files get full write + delete; ephemeral (per-boot) is acceptable.

## Target layout

```
/                 vfsd  ata  format  mount  rm  cp  mkdir  rmdir  mem
/games/           snake  tetris  conway
/services/        randomd  echo  bulk  pong  ping  inputd  ps2d  atad
/tests/           threads  waitset  stress  vfs
/bench/           bench  pi
```

The four directories are seeded by `vfsd` at volume creation and are plain rxfs directories — deletable, and usable as mount points like any other.

## Current state (verified)

- The root volume is **rxfs on a `MemoryDevice`**, not the `ramfs` crate: `VfsStore::new()` formats a fresh 8 MiB volume (2048 blocks, 512 inodes) every boot (`apps/vfsd/src/fs.rs:149`). `fs/ramfs` stays unwired — it cannot do directories.
- The ROM arrives as regular files: `vfsd` reads the image via `RomRead` and `attach_rom()` creates each entry at `/name` (`fs.rs:251`).
- ROM read-only is **daemon policy only**: the `rom_names` name-set rejects `write()` (`fs.rs:344`) and `delete()` (`fs.rs:396`) with `NotPermitted`. The engine is already fully writable.
- **Nested paths are blocked at the daemon edge:** `route()` (`fs.rs:221`) answers `NotFound` for any multi-segment path whose first segment is not a live mount, so nothing below a root-volume directory is reachable. rxfs (`path.rs`, `resolve_prefix`), the wire (`valid_path` accepts nested paths ≤ 57 bytes total), the CLI tools, and error mapping all already handle nesting.
- Boot services (`vfsd`, `inputd`, `ps2d`, `atad`) launch from `include_bytes!`-embedded ELFs, never through the VFS — the ROM move does not touch the boot roster or the kernel.
- `RomImage` names are arbitrary strings ≤ 57 bytes; only `attach_rom` interprets them, and only its `wire::valid_name` check rejects `/`.
- The shell launcher stats/opens `cmd.name` verbatim; `Command::parse` splits on whitespace, so `games/snake` flows through untouched — the launcher needs no change once `route()` is fixed.
- The launcher's size-bounded image read already turns a mid-session mutated ELF into a "truncated image" refusal instead of execution — that guard covers accidental ROM edits.

## Commits (each independently green)

### C1 — Writable ROM files
`apps/vfsd/src/fs.rs`: delete `rom_names` (field, insertion in `attach_rom`, guard in `write`, guard in `delete`). Keep `rom_attached` (the `AlreadyAttached` guard).
Unit tests: flip `write_to_rom_file_is_not_permitted_even_when_empty` and `delete_rom_file_is_not_permitted` into positive assertions; add partial-overwrite-read-back and delete-then-recreate-under-the-same-name tests. Keep `open_rom_with_create_is_already_exists` (generic engine behavior).
No wire, kernel, treaty, or xtask change.

### C2 — Nested paths reach the root engine
`apps/vfsd/src/fs.rs` `route()`: when the first segment is not a mounted name, fall through to `Target::Root` with the full path (mounts keep priority, preserving overlay semantics).
Rewrite `unknown_mount_prefix_is_not_found_for_every_operation`: unknown prefixes still answer `NotFound`, now from rxfs (missing parent), not from the router; add unit tests proving `mkdir games/deep`, `create games/file`, `list_dir games`, read/write/delete, and `rmdir` all work on root-volume paths, while a mounted name still shadows its root directory.

### C3 — Seed the organization directories
`VfsStore::new()` (`impl` block at `fs.rs:148`): after `FileSystem::format`, `mkdir` `/tests`, `/services`, `/games`, `/bench` on the engine before handing it to `from_engine`. Seeding lives in `new()` only — `from_engine` (used by tests) stays unseeded.
Unit test: fresh store lists exactly the four seeded names; seeded names are real directories (`stat` → `Directory`).

### C4 — Ingest ROM entries as paths
`attach_rom()`: validate entries with `wire::valid_path` (error variant rename `InvalidName` → `InvalidPath`), duplicate-check on the full path, and create missing parent directories (mkdir-each-prefix, ignoring `AlreadyExists`) before creating each file.
Unit tests: `games/snake`-style entries land nested; parents appear implicitly; a file-vs-directory path collision (`services` as both dir and file target) fails as `Duplicate`/`AlreadyExists` and stages nothing.

### C5 — Taxonomy + all consumers (single commit; splitting breaks CI)
- `xtask/src/main.rs` `pack_rom`: express the taxonomy as one named const per category (`ROM_GAMES`, `ROM_SERVICES`, `ROM_TESTS`, `ROM_BENCH`, `ROM_ROOT_TOOLS`), each listing the app names that pack under that directory; `pack_rom` joins category to name to build the ROM entry path (on-disk artifacts in `target/rosx-user/release/` keep their bare names; only the ROM entry name changes). A compile-time-counted check (mapped total == `USER_APPS` len, asserted in `pack_rom`) fails the pack if an app is unassigned, so a new app cannot silently reach the root. Assert every mapped name passes `wire::valid_path` so a bad roster fails at pack time, not at boot.
- `apps/vfs/src/main.rs`: `stat randomd rom intact` → stat `services/randomd`; add nested checks pinning the new powers — write+read-back bytes into a ROM file (`services/randomd` append at EOF via reopen), create/write/delete `tests/scratch.txt` inside a seeded directory.
- Integration launch lines: `app_lifecycle` (`games/tetris`, `games/snake`), `bench_alloc` / `bench_allocator` / `bench_baseline` (`bench/bench ...`), `elf_arguments` + `shell_command_arguments` (`bench/pi ...`), `ipc_ping` (`services/ping`) + `ipc_wake_latency` (`services/ping burst`), `waitset_or` (`tests/waitset`), `stress_mixed_preemption` (`tests/stress mixed`), `ipc_vfs` (`tests/vfs` + the two renamed/vfs-client lines in `EXPECTED_OK`).
- `boot_and_shell::shell_ls_and_unknown_command`: root `ls` now asserts directory names (`games`, `services`, ...); add `ls games` expecting `tetris`.
- Untouched by design: `ipc_random` (builtin), `kstats_mem` (`mem` at `/`), `ata_*` (`ata`/`format`/`mount` at `/`), `vfs_hdd_flow`, `iso_boot` (prompt-only), `arch/*/main.rs` boot rosters, `system/src/rom.rs` (no format change), the shell and its launcher.

### C6 — Documentation
- `AGENTS.md`: VFS architecture paragraph (root-volume nesting, seeded directories, writable ROM, relative-path launching), the `rom/` bullet, and the Build & Run example (`rose> games/snake`).
- **ROM roster pattern (the guide for future changes):** a dedicated `AGENTS.md` subsection stating the rules `pack_rom` encodes — every app in `USER_APPS` must carry exactly one category assignment (`ROM_GAMES` / `ROM_SERVICES` / `ROM_TESTS` / `ROM_BENCH` / `ROM_ROOT_TOOLS`); how to choose (interactive amusements → games, IPC daemons that answer a bound service name → services, self-checking suites → tests, measurement harnesses → bench, user-facing command tools → root); ROM entry names are paths that must pass `wire::valid_path` (total ≤ 57 bytes, no leading/trailing slash); shell launching is by relative path (`games/snake`), so adding an app to a category changes its prompt name and its integration-test launch lines together; boot daemons (`vfsd`, `inputd`, `ps2d`, `atad`) are additionally embedded in the kernel via `include_bytes!` and their boot roster is independent of the ROM taxonomy; the seeded root directories (`tests`, `services`, `games`, `bench`) are created by `vfsd`, so a new category needs a seed entry in `VfsStore::new()` alongside its `ROM_*` const. This section is the pattern's single home: the code carries the data, the document carries the rules (the no-comments law keeps the rationale out of `xtask`).

## Verification

1. `cargo xtask test --skip-integration` (vfsd/fs-core/rxfs/system unit tests; `cargo test --workspace`).
2. `cargo xtask build` and `cargo xtask build x86_32`; `cargo xtask rom` proves the roster check by temporarily removing one mapping (expected: pack fails on an unassigned app) — rehearsed once at C5, not left in code.
3. QEMU (`cargo xtask run`): `ls` shows 9 tools + 4 directories; `ls games` shows the games; `games/snake` launches; `vfs` (`tests/vfs`) suite green; `rm games/snake` then `ls games` confirms deletion; `mkdir games/sub` + `cp ... games/sub/...` confirms nesting end-to-end.
4. Integration: the eight touched tests plus `vfs_hdd_flow`, on both arches (`ROSX_ARCH=x86_32` suite per AGENTS).

## Notes and accepted consequences

- **No persistence:** the root volume is reformatted every boot; ROM edits and files vanish on reboot. A disk-backed persistent root is a separate future effort (boot-time `mount` from ATA).
- **ROM edits are live:** overwriting `games/snake` genuinely changes what the next `games/snake` run executes (that is the point); a half-written ELF fails as "truncated image" at the launcher's size-bounded read.
- **Mount overlay unchanged:** `mount 0 games` over the seeded directory works exactly like today's mount-point rules, and `rm`/`rmdir` keep refusing a live mount point.
- The longest mapped path is 16 bytes (`services/randomd`), far under the 57-byte wire frame.
- Inode budget: 512 inodes against 26 apps + 4 directories + user files — ample.
