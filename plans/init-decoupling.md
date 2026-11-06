# Init Decoupling: a Manifest-Driven `/init` Retires the Kernel Boot Roster

> **Status:** **Done** (PR #128) — shipped 2026-11-06 per the owner decision:
> manifest lives in the ROM as `init.lst`; handoff is the minimal-change
> variant (prefer disk, fall back to ROM). Evidence: the Evidence section below.
> Written 2026-11-06 against `boot-packaging` (PR #127, arc 3 of the FAT program).
> **Repo law applies** (AGENTS.md): no comments in Rust, no commit-subject prefix,
> one commit per step, unit tests per module, kernel = mechanism / policy userspace.

---

## Goal

Today both `arch/*/main.rs` hardcode the boot roster: nine user ELFs scheduled by
name from the ROM (x86_32) / ten (x86_64, same set). The kernel is the policy
engine of the boot — it knows which daemons exist, in what order, and grants the
ATA ports to a task it picks by hand.

The goal is the Unix `/init` split, kept honest to the project's own law ("kernel
is mechanism, policy is userspace"):

1. The kernel boots **only**: `kstatsd` (a kernel-side `FunctionTask` — stays),
   `atad` (see I2 — port grants are a boot authority not yet delegable), and
   **`init`** — one new, tiny user ELF, extracted from the ROM like today.
2. `init` reads the ROM file **`init.lst`** — the boot manifest — and brings up
   every other service, then performs the **handoff**: exec the shell from the
   disk if it is there, else from the ROM, then exit.
3. The service list becomes **data**: a different machine is a different
   `init.lst`, not a kernel recompile. CI's manifest keeps the IPC demo daemons;
   a production manifest drops them.

The ROM keeps its second role untouched: `vfsd` still ingests it as the root
volume's app library (the 696 KiB distribution story is a separate, parked
installer arc — see Follow-ups).

## Verified source facts (2026-11-06)

- Boot roster today (`arch/x86_32/src/main.rs:66-95`, x86_64 mirror):
  kstatsd `FunctionTask`, then `new_elf_task` for randomd, echo, bulk, pong,
  vfsd, inputd, ps2d, atad (via `schedule_with_task_handle` + two
  `platform::portio::grant` calls: `0x1F0-0x1F7`, `0x3F6`), then shell.
- `kernel::rom::file_in` (`kernel/src/rom.rs`) — the name lookup both ports use;
  init will use the user-side equivalent (`system::rom::RomImage::parse` over a
  `rom_read` loop) exactly like `vfsd::auto_mount` does (`apps/vfsd/src/main.rs:97`).
- Syscalls init needs **all exist**: `RomRead = 27`
  (`usrlib/src/syscall.rs:221`), `LoadElf = 10` with `load(&[u8])` and
  `load_with_args(&[u8], &[String])` (`usrlib/src/syscall.rs:35-58`). The kernel
  handler (`kernel/src/syscall.rs:84-101`) names every LoadElf child `"ELF"` and
  passes the future-token handle up; each user LoadElf starts a fresh memory
  group, so children of init outlive it (AGENTS memory-ownership law).
- **Port grants are kernel-internal**: `GrantTable::grant` is `pub(crate)`
  (`kernel/src/platform/portio.rs:51`), keyed by `TaskHandle`; the `LoadElf`
  syscall path grants nothing, and no user API can. `TaskHandle` is
  `(index, generation)`-tagged (`kernel/src/handle.rs:44-50`), so stale grant
  entries can never alias a recycled handle — at worst they leak one of the 8
  table slots. This is why `atad` stays a boot program (I2).
- VFS dial + retry is inherited: `Vfs::connect()` retries 50×100 ms (arc-2 B9);
  the shell's `await_filesystem()` (`apps/shell/src/shell.rs:71`) stays exactly
  as is — the prompt guarantee does not move.
- Routing for the handoff lookup exists: a path's first segment is looked up in
  the mount table, so `hdd/shell` routes to the volume root's `shell` once the
  fstab entry `hdd0 ATA 1 /hdd` has mounted the disk; unmounted/absent answers
  `NotFound` (arc-2 routing law). A read-only `stat` query answers
  path → kind + size with zero open slots.
- ROM today: 29 records (28 ELFs + `fstab`), ~1075 KiB. Boot-critical daemons:
  vfsd 241K, atad 44K, inputd 23K, ps2d 22K, shell 47K.
- xtask special-cases `shell` in exactly three places — apps build
  (`xtask/src/main.rs:52`), `pack_rom` (108-117), check-isa ELF list (349).
  `init` joins all three; `init.lst` joins pack_rom beside `DEFAULT_FSTAB`.
- Era/QEMU media ride the ROM as one Multiboot module / ramdisk — **zero media
  changes**: `init` and `init.lst` are ROM records like any other.

## Design rules (locked)

- **I1 — The manifest is a ROM file `init.lst`**, plain text, `#` comments,
  malformed lines skipped-and-counted (the `fstab` precedent, 17-test grammar).
  Grammar, three verbs, one per line:
  `service <rom-name>` — spawn this ROM ELF, in file order;
  `handoff <vfs-path>` — the disk handoff target (default file ships `hdd/shell`);
  `fallback <rom-name>` — ROM name to exec if the handoff misses (`shell`).
  Duplicate `handoff`/`fallback` → skipped-and-counted. No handoff line at all is
  legal (headless manifest: init spawns services and exits).
- **I2 — The kernel boots `kstatsd` + `atad` + `init`, and nothing else.**
  Port grants require boot authority (kernel-internal table keyed by task
  handle; delegating them is a capability syscall this arc deliberately does not
  invent — see Follow-ups F1 with a full sketch). `atad` must therefore NOT
  appear in `init.lst` (the default manifest simply doesn't list it; policing
  grammar against it would be policy theatre).
- **I3 — Zero kernel/wire changes.** init lives on `RomRead`, `LoadElf`
  (`load`/`load_with_args`), the VFS client, and `println`. No new syscall,
  no ABI frame change.
- **I4 — Handoff = prefer disk, fall back ROM** (owner choice: the minimal-
  change variant — ROM shell stays, so every existing test and every first-boot
  keeps working; disk support is purely additive). Sequence: dial VFS (bounded
  retries), `stat hdd/shell`; on success read the bytes (size-capped, reusing
  the shell launcher's truncated-file guard pattern) and `load_with_args`; any
  failure path → `load_with_args(rom.shell)`. Then init exits — no supervision
  in v1 (F2 owns respawn).
- **I5 — Spawned services keep the kernel's `"ELF"` task name in v1** (the same
  class every shell-launched task already has — no new regression). The `[INIT]`
  log lines carry the correlation. A named-`LoadElf` rides F1's frame change.
- **I6 — Ordering = manifest order, spawn-and-go.** No new sync primitives: the
  boot stays retry-tolerant by the daemons' dial-retries and `vfsd`'s
  bind-after-automount gate (B9), exactly as the kernel's fixed order relied on
  today. The default manifest preserves today's relative order (minus atad,
  which the kernel boots before init — earlier than today, harmless: clients
  dial-retry `ATA`).
- **I7 — init never panics on boot data.** Missing ROM record for a service →
  `[INIT] {name} skipped: no rom record`, continue (the fstab skip philosophy —
  a broken boot is caught by the suite, not by halting the machine). Final
  failure (no handoff AND no fallback ELF) → log `[INIT] handoff failed`, exit;
  the system idles with whatever did start, keyboard alive.
- **I8 — The default `init.lst` mirrors today's boot exactly**, so all suites
  pass unchanged:

  ```
  service randomd
  service echo
  service bulk
  service pong
  service vfsd
  service inputd
  service ps2d
  handoff hdd/shell
  fallback shell
  ```
- **I9 — Observability strings (tests pin these, exactly):**
  `[INIT] init.lst: {n} services`, `[INIT] {name} skipped: ...`,
  `[INIT] handoff hdd/shell`, `[INIT] fallback shell`, `[INIT] {n} skipped`.
  They precede `rose>` by construction (I6 order; init spawns shell last).

## Steps

1. **`apps/init` — manifest grammar.** New workspace member (usrlib + system
   deps only); `manifest.rs` pure parser + `#[cfg(test)]` grammar suite
   (comments, blank lines, unknown verb → skip-count, duplicate handoff →
   skip-count, empty services legal, CRLF-safe like fstab). Host-testable;
   nothing wired yet. *Gate:* `cargo test -p init`.
2. **`apps/init` — the boot body + xtask wiring.** `main.rs`: ROM loop
   (`rom_read` chunks → `RomImage::parse` → find `init.lst`) → parse → service
   loop with skip semantics → handoff/fallback per I4 → exit. xtask: `-p init`
   in apps build, `init` + `INIT_LST` const (I8 content) in `pack_rom`,
   `elfs.push("init")` in check-isa. *Gate:* unit tests; `cargo xtask rom`
   reports **31 records** (29 ELF + fstab + init.lst); `cargo xtask build`
   green both arches (init still not booted).
3. **x86_32 cutover.** `kernel_main`: kstatsd + atad (existing `grant` pair
   unchanged) + `roster_elf(rom, "init")` — the other seven schedules deleted.
   *Gate:* `ROSX_ARCH=x86_32` `boot_and_shell` 4/4, `ipc_ping`, `vfs_hdd_flow`.
4. **x86_64 cutover.** Mirror of step 3. *Gate:* `boot_and_shell` 4/4,
   `ipc_bulk`, `vfs_hdd_flow` (the zero-ritual session-2 promise must hold).
5. **`init_handoff` integration test.** Session A (no disk): expect
   `[INIT] fallback shell` then `rose>`. Session B (fresh tempdir hdd):
   `format hdd0` → `mount hdd0 /hdd` → `cp shell hdd/shell` (ROM volume →
   disk). Session C (same disk): automount answers first, expect
   `[INIT] handoff hdd/shell`, no `fallback` line, `rose>` works
   (`ls /hdd` shows `shell`). *Gate:* green on x86_64 and x86_32.
6. **Docs + era rehearsal.** AGENTS.md (boot-roster story in both arch
   sections, `rom/` line: ROM carries the manifest, `apps` list + `init`),
   plan evidence log. Era HDD rebuilt from the new ROM, one QEMU `-cpu 486
   -m 8` boot: `rose>` + the `[INIT]` lines prove the manifest rides the
   module untouched.
7. **Full battery + PR.** `cargo xtask test`; both integration suites;
   `check-isa x86_32` (now 29 ELFs); clippy on touched crates; push, PR stacked
   on `boot-packaging`.

## Evidence (2026-11-06)

- Steps 1-2: `cargo test -p init` 19 grammar tests green; `cargo xtask rom`
  reports **31 records**; `cargo xtask build` green on both arches.
- Steps 3 + 4: both cutovers green on `boot_and_shell` (4/4), the disk flows,
  and the IPC gates. Boot windows show `[INIT] init.lst: 7 services`, the
  daemon banners, `[INIT] fallback shell`, then `rose>` — the pinned I9 order.
- **ipc_ping race found and closed (pre-existing).** The branch initially
  failed `ipc_ping` ~1/12: the last-round `PONG 65536 matched ok` could land
  *after* the shell repainted `rose> ` (the prompt has no newline), gluing the
  lines so the test's `starts_with` count missed it (`rose> PONG 65536 ...`).
  The kernel has never serialized concurrent prints — `ipc_bulk` already
  counts with `contains` for exactly this shape; IRQ-masking `print` does not
  help (it is an ordering race, measured 4/24 failing with it). Fix (owner
  choice): **pong logs the round before answering** — the `PONG` print
  completes strictly before any client can wake, so no later prompt can precede
  it. Measured 0/36 green after the fix (base had been winning the same coin
  flip 0/36 by timing luck, not by guarantee).
- Step 5: `init_handoff` green on both arches: diskless boot answers
  `[INIT] fallback shell` and never `handoff`; after `format`+`mount`+
  `cp shell hdd/shell`, the rebooted machine answers `[INIT] handoff hdd/shell`
  with no fallback line, and `ls /hdd` shows `shell`.
- Step 6 era rehearsal: HDD rebuilt from the new ROM, one QEMU
  `-cpu 486 -m 8` boot reaches `rose>` with `[INIT] init.lst: 7 services` and
  `[INIT] fallback shell` — the manifest rides the Multiboot module untouched.
- **Era OOM found (pre-existing, F3 territory).** On the `-m 8` control
  (~5 MiB usable) the boot panics with an allocation failure inside `vfsd`'s
  whole-ROM `mount_rom` copy — and it does so **on base too** (measured on
  `fdada19`): a ~1 MiB ROM does not fit a 5 MiB era box. `rose>` still appears
  (`await_filesystem` gives up quietly), which is why earlier era rehearsals
  that only checked the prompt missed it. The honest fix is F3 (shrink the
  distribution out of the ROM), not this arc; init's own ROM copy follows the
  plan's `vfsd::auto_mount` precedent and can stream record-by-record when F3
  lands (RomRead is offset-addressable).

## Follow-ups (documented, NOT this arc)

- **F1 — Port delegation frees `atad` (and names children).** Sketch, ready to
  spec: new syscall `LoadElfService` — ELF cell + name + `[(base, end)]` blob;
  both archs pass their delegatable ranges to the kernel at boot (today's
  `0x1F0-0x1F7`, `0x3F6` become a data table registered from arch, keeping port
  values arch-owned); the kernel stashes init's `TaskHandle` at boot
  (`register_init`) and accepts the syscall **iff the caller is that handle**;
  grants write through the existing `GrantTable` (capacity 8 → 16; generation
  tags already defend against stale aliasing). The same frame fixes I5.
- **F2 — Supervision:** init waits on child completion futures (the
  `TaskCompletionFuture` already exists) and respawns with backoff; the manifest
  grows a `respawn` flag.
- **F3 — Distribution shrink:** the 696 KiB app library leaves the ROM once a
  provisioned-disk story exists (fat-disk packs it today; a first-boot installer
  or a pre-baked app disk is the enabler). Until then the ROM is the distribution
  and trimming it would starve `rose>` — this is the arc boundary the owner
  framed and this plan honours.
- **F4 — Kernel cmdline override** (`Multiboot info` `cmdline` @16, already
  decoded territory) for `cargo xtask run --manifest=...` developer ergonomics.
