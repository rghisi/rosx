# Boot Auto-Mount: the Disk Is There When the Prompt Appears

> **Status:** Done (PR #126, 2026-11-06) (arc 2 of 3). Branch
> `boot-automount`, stacked on `fat-write` (PR #125). Arc 1 made FAT
> two-way; this arc removed the per-boot manual ritual the persistence
> story still demanded. Arc 3 is the parked `boot-packaging.md` (P1).

## Goal

Today every boot — both arches, every test, every real run — repeats the
same liturgy before the persistent disk is reachable: `mkdir /hdd`,
`attach <id> ATA 1`, `mount <id> /hdd`, because the attach registry is
in-memory in `vfsd` and dies with it. After this arc, `vfsd` reads a tiny
mount table — **the fstab, a file in the ROM image** — between ROM ingest
and service bind, and attaches + mounts each listed drive by itself.
`cargo run` on a formatted disk boots straight to `ls /hdd` working.

## Facts this plan rests on (verified 2026-11-06)

- `vfsd` startup (main.rs) is exactly `new → mount_rom → bind/run`, and
  `mount_rom` already holds the full ROM bytes in hand — the insertion
  point and the config source meet for free.
- The ROM format (`system::rom`) is a generic name→blob table with no
  roster restrictions; vfsd already depends on `system`. `xtask pack_rom`
  is one `files.push(...)` away from shipping a `fstab` entry.
- **Service startup races are already absorbed**: `block-protocol` and
  `vfs-protocol` clients both retry connect 50×100ms, and `atad` binds its
  `ATA` service even when a drive is absent (probe fails per-drive, fast).
  `attach_device` runs its duplicate-alias and duplicate-(service, device)
  guards *before* dialing, so unit tests never hang on connect.
- `mount_device` requires a bare `valid_name` directory that already
  exists as a root-volume directory; rxfs `mkdir` on an existing name
  answers `AlreadyExists` (pinned test) — the auto pass tolerates exactly
  that.
- The integration suite's `boot_and_shell` listing asserts are
  containment-style, so `fstab` and `hdd` appearing in the root listing
  breaks nothing.
- Existing scripts to update: `vfs_hdd_flow` (session 1 zeroed disk,
  session 2 persistence), `fat_exec` and `fat_write` (FAT16 volume). All
  three hand the drive to the same registry under aliases `ata0`/`fat0` —
  once `hdd0` claims `(ATA, 1)` at boot, their manual `attach` answers
  `Busy`, which is the registry's honest single-owner rule doing its job.

## Locked decisions

- **B1 — the fstab is a ROM file named `fstab`**, plain text: one entry
  per line, whitespace-separated `alias service drive dir`; `#` comments
  and blank lines skipped; a leading `/` on `dir` is accepted and
  normalized to the bare name `mount_device` wants. It ships as a regular
  ROM file (visible in `ls`, write/delete-denied by the existing ROM name
  set — the table is tamper-evident without inventing machinery).
- **B2 — default content packs exactly one line: `hdd0 ATA 1 /hdd`**, the
  alias/`/hdd` convention the AGENTS flow already uses. One drive, one
  mount point; nothing else auto-mounts. The alias is **not** special in
  the daemon: `hdd0` lives in the config file, not in code.
- **B3 — the pass runs after ROM ingest, before `ipc_bind(VFS)`**. Clients
  asking for VFS during the pass simply retry-connect (5s budget), which
  every client already does; in return, no client can ever observe a
  half-populated registry or race the auto-attach.
- **B4 — boot never fails because of a disk.** The pass verb is **skip**:
  an entry that cannot attach, mkdir, or mount yields
  `[VFSD] automount <alias>: <step> skipped: <error>` and the next entry
  — never a panic, and never the word "failed" for a drive the machine
  simply does not have (the healthy-window tests forbid that string, and
  an absent optional disk is a healthy state). Contrast ROM ingest, which
  does panic (no root without the ROM; a disk is optional).
- **B5 — mount-point creation is the pass's job**: after a successful
  attach it creates the directory (tolerating `AlreadyExists`), then
  mounts. An attached-but-unformatted volume leaves the directory in
  place so the human flow is just `format hdd0` → `mount hdd0 /hdd`.
- **B6 — malformed lines are skipped and counted**, reported as one
  `[VFSD] fstab: N skipped` line when non-zero; valid siblings still
  mount. A typo costs a disk, not a boot.
- **B7 — `format` does not auto-mount afterwards** (scope discipline:
  first format is a one-time human act; every later boot mounts by
  itself).
- **B8 — tests follow the new world.** `vfs_hdd_flow` shrinks (session 2
  does zero mounting — that IS the arc's headline proof); `fat_exec` and
  `fat_write` rename `/fat` → `/hdd` and drop the dance; the old manual
  `attach` lines become the *negative* assertion (a claimed drive answers
  `Busy`).
- **B9 — the prompt means the filesystem is ready.** The shell dials VFS
  once (`connect` + `disconnect`) before printing its first prompt. The
  boot-time automount log line otherwise lands on the console whenever
  the scheduler pleases — an early run proved it slicing
  `rose> hel[VFSD] automount ...` straight through the keystroke-echo
  test. With B3 (bind after the automount pass) plus B9, the ordering is
  guaranteed: `[VFSD] automount...` always precedes `rose>`, the console
  after the prompt is clean, and `ls /hdd` at the first prompt is
  meaningful. A dead `vfsd` costs the shell one 5s connect budget, then
  the prompt appears anyway and per-command errors stay as they were.

## Milestones

1. **`vfsd::fstab` module**: `FstabEntry { alias, service, drive, dir }`
   + `parse(bytes) -> (entries, skipped)` with the B1 grammar, plus a
   unit suite (good lines, comments, blanks, CRLF, junk lines of each
   shape, over-long tokens, duplicate aliases left for the registry to
   police).
2. **The boot pass** in `main.rs`: locate the `fstab` entry in the ROM
   image (`system::rom::RomImage`), parse it, then per entry
   attach → mkdir (tolerate `AlreadyExists`) → mount, with B4/B5/B6
   logging. Thin glue over the same `ActiveStore` methods the wire
   dispatch uses — no new store surface beyond a `mkdir`-tolerant
   sequencing helper if one reads better.
3. **`xtask pack_rom` ships the fstab** (B2). Re-packed ROMs on both
   arches; kernel `build.rs` gates stay honest.
4. **Tests follow (B8)**: rewrite the three flows, add the automount-log
   line as the boot-sync point, and assert `attach list` shows the
   auto-attached `hdd0` before anyone types a mount command.
5. **Full battery** (both arches): unit suites, both integration suites,
   `check-isa x86_32`, clippy on touched crates; then push + PR.

## What this arc deliberately does not do

- **Partitioning / multiple entries' worth of media** — the default table
  has one line because there is one convention drive; the *format*
  supports the fleet P2 will grow into.
- **Auto-format** (B7), **fstab editing from user space** (the file is a
  ROM entry; changing the table means rebuilding the ROM — honest for a
  boot-time policy), **unmount at shutdown** (there is no shutdown).
- **P1 boot packaging** (arc 3).

## Evidence (2026-11-06)

- **Units:** `vfsd` 98 (81 registry/engine + 17 new `fstab` grammar);
  full `cargo xtask test` battery exits 0 — 70 suites, zero failures.
- **x86_64 integration:** the rewritten trio green — `vfs_hdd_flow` 49s
  (session 1: `mount skipped: NotFormatted` at boot, `attach ata1 ATA 1`
  → `Busy`, `mkdir /hdd` → `AlreadyExists`, then `format hdd0` →
  `mount hdd0 /hdd`; session 2: boots formatted media, sees
  `[VFSD] automounted hdd0 at /hdd`, and `ls /hdd` works with **zero
  mount commands**), `fat_exec` ~19s (no dance, execs `hdd/mem`),
  `fat_write` ~40s (writes + reboot persistence + mtools byte proof).
  `boot_and_shell` 4/4 — the keystroke echo passes because the prompt
  gate (B9) keeps the console clean.
- **x86_32 integration:** 26 suites, zero failures (`-cpu 486`),
  `check-isa x86_32` clean. The run caught a latent harness race:
  `ipc_ping` snapshotted its window at the last *client* line while the
  PONG server's print can legally land after its reply (visible ordering
  flip in the 64-byte round); the window now waits for both terminal
  lines — boot-timing shifts must never masquerade as ping bugs.
- **Clippy:** zero warnings pointing at `apps/vfsd` or `apps/shell`.
- **Boot-log phrasing earned its shape on real failures:** the daemon's
  `mkdir` wire convention is no-leading-slash (the pass learned via
  `mkdir skipped: InvalidPath`), and the healthy-window tripwires
  (forbidden string `fail`) forced B4's *skip* verb — an absent optional
  disk is a healthy boot, and the logs now say so.

## Residual risks and mitigations

- **A disk that answers the probe slowly** (real hardware) could outlast
  the connect budget at first attach: attach logs and the human flow
  (`attach` CLI) still works after boot — the automation is a convenience
  layer over the same registry, not a replacement for it.
- **Alias collision** between fstab and an operator's later manual
  attach: surfaces as `Busy` with the fstab alias already in
  `attach list` — visible, explainable, one edit away.
- **Boot-order regressions** (vfsd delaying bind too long): the pass is
  bounded by the clients' 5s connect budgets per entry and boots with no
  disk at all in ~one probe round-trip; `boot_and_shell` keeps the boot
  budget under watch.
