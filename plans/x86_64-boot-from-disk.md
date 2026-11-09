# x86_64 Boot-from-Disk Parity — FAT main volume, slim ROM, identical init plan

> **Status:** Executed 2026-11-09 on branch `x86_64-boot-from-disk` (milestones M1–M5 below,
> execution ledger in §6). Written 2026-11-09 at worktree `quiet-cactus`, HEAD `1df0dca`.
> **Goal:** make x86_64 boot and use the disk volume the way x86_32 does, as close as the two
> kernel-loading mechanisms allow: the main disk is a FAT volume carrying every user app at `/`
> plus `/boot/kernel.elf` + `/boot/rom.img`; the x86_64 ROM roster shrinks to the same
> `BOOTSTRAP_APPS` set x86_32 ships; the init boot plan becomes x86_32's, byte for byte.
> **Method:** every code fact below is cited to a path (and line) re-verified against this tree;
> every runtime fact was measured on this machine (QEMU 10.2.1, `nightly-2026-10-03`). Measured
> runtime observations are marked **[O]**. Facts this plan cannot prove are marked **unverified**
> and routed to the milestone that must prove them.
> Repo law applies throughout: `kernel/` stays `no_std` and arch-agnostic, **no comments ever**
> (assembly excepted), unit tests for kernel modules, no test-only production surface,
> `cargo build --workspace` stays forbidden, commit subjects are bare imperative (no prefix).

---

## 0. Goal & Non-Goals

### Goal
- `cargo xtask build x86_64` writes `target/rosx-boot-disk.img`: the same FAT boot-disk recipe
  x86_32 already runs (`rxpart`-stamped MBR, one FAT16 slice, all `USER_APPS` ELFs at the FAT
  root, `/boot/kernel.elf` + `/boot/rom.img` staged under `/boot`).
- `cargo xtask run x86_64` boots with that FAT volume at the *primary master* IDE slot, so
  blockd's curated namespace is `ata0d0` with slice `ata0d0p0` — the exact device name the boot
  plan speaks — and the persistent 50 MiB host disk stays `ata0d1`.
- The x86_64 ROM carries only the bootstrap eight (`atad, blockd, attach, mount, vfsd, inputd,
  ps2d, shell`), exactly like x86_32.
- The x86_64 boot plan is x86_32's 12-step plan verbatim: ROM bootstrap stack, `run rom:mount
  ata0d0p0 /`, disk-sourced `randomd`/`echo`/`pong`, `exec /shell` falling through to
  `exec rom:shell`.
- The integration harness gives x86_64 sessions the same full-apps FAT master x86_32 sessions
  get, so both arches run every suite root-mounted on the disk volume.
- CI builds (and the disk jobs boot-test) this without new tooling gaps.

### Non-Goals
- No Multiboot port of the x86_64 kernel. The kernel stays on the `bootloader` crate
  (`BiosBoot`); its disk image remains the kernel carrier. The on-disk `/boot/kernel.elf` +
  `/boot/rom.img` stay forward-looking staging artifacts — the identical treaty x86_32 already
  has with its QEMU `-kernel` flow.
- No GRUB (Legacy or 2) boot media for x86_64; the era-media family (`cargo xtask era-image`)
  stays x86_32-only.
- No partition-table writer and no change to `blockd`/`rxpart` semantics.
- No x86_32 behavior change of any kind (its topology is the reference this plan copies).
- No `disk_boot`/`iso_boot` arch-guard widening to x86_64 in this iteration (the x86_64 kernel
  cannot be *loaded* from its own FAT disk yet — see Non-Goal 1).

---

## 1. Verified baseline facts

### Code facts (cited against `1df0dca`)

- **F1** The x86_64 boot plan is all-ROM: `arch/x86_64/src/main.rs:80-89` lists eight bare
  (implicit-ROM) `service` steps plus `exec shell`, no mount step, no disk hand-off. The ROM
  volume is its filesystem.
- **F2** The x86_32 boot plan is the reference: `arch/x86_32/src/main.rs:60-72` —
  `service rom:atad` → `service rom:blockd` → `run rom:attach ATA` → `service rom:vfsd` →
  `service rom:inputd` → `service rom:ps2d` → `run rom:mount ata0d0p0 /` →
  `service /randomd` → `service /echo` → `service /pong` → `exec /shell` → `exec rom:shell`.
- **F3** Rosters live in `xtask/src/arch.rs`: `USER_APPS` (line 5, 31 apps) and `BOOTSTRAP_APPS`
  (line 7, the eight). x86_32 ships `BOOTSTRAP_APPS` in ROM (`arch.rs:45`); x86_64 still ships
  the full `USER_APPS` (`arch.rs:32`).
- **F4** The FAT boot disk is an arch-spec flag: `ArchSpec::fat_boot_disk` (`arch.rs:18`, false
  for x86_64 at `:31`, true for x86_32 at `:44`) gates `create_boot_disk`
  (`xtask/src/main.rs:109-113`). The recipe (`xtask/src/boot_disk.rs:28-40`) stamps a 64 MiB
  image's MBR through `rxpart::mbr::encode_mbr` (bootable FAT16 0x06 slice at LBA 63), then
  `mformat`/`mmd`/`mcopy` put every `USER_APPS` ELF at the FAT root and kernel + ROM under
  `/boot`. The path formula is `target/<kernel_dir_name>-boot-disk.img`
  (`boot_disk.rs:22-26`) — for x86_64 that is `target/rosx-boot-disk.img`, a new file name, no
  collision with the existing `target/rosx/release/rosx-x86_64.img` carrier image.
- **F5** QEMU wiring today: `x86_runner::qemu_command` attaches the `DiskImage` medium as a bare
  `-drive format=raw,file=…` (`arch/x86-runner/src/lib.rs:197-200`) — implicit *primary master*,
  which is why the bootloader's own MBR surfaces as `ata0d0p1/p2` (the documented footgun) — and
  the persistent HDD lands at `if=ide,index=1` (`lib.rs:246-262`). The multiboot kernel rides
  `-kernel` (`lib.rs:201-203`) and consumes no IDE slot. `cargo xtask run` attaches the FAT boot
  disk at `index=0` **only for the multiboot medium** (`xtask/src/main.rs:76-85`).
- **F6** The kernel's ATA driver owns the **primary channel only**: the grantable port pool is
  `0x1F0–0x1F7` + `0x3F6` (`arch/x86-generic/src/port.rs:4`, `ATA_PORT_POOL`). Drives on the
  secondary channel (0x170/0x376) are physically unreachable to `atad`, hence invisible to
  `blockd` and the whole guest namespace.
- **F7** The `bootloader` crate 0.11.17 BIOS chain honors the BIOS-supplied drive number end to
  end: `boot.s:37-43` passes `DL` through the INT 13h extension probe and `push dx` into
  `first_stage(disk_number)` (`src/main.rs:29`), which hands it to stage 2
  (`main.rs:64-68`), whose disk access issues `int 0x13` with `in("dx") disk_number`
  (`bootloader-x86_64-bios-stage-2-0.11.17/src/dap.rs:38-54`). Nothing hardcodes 0x80. (Registry
  sources under `~/.cargo/registry/src/`.)
- **F8** The integration harness attaches its full-apps FAT master only for the multiboot
  medium: `attach_multiboot_master_disk` (`tests-integration/src/qemu.rs:335-349`) is gated on
  `BootMedium::MultibootKernel`. The master image builder (`tests-integration/src/fat_disk.rs`)
  is already arch-neutral: it copies every dot-free file from `Arch::current()`'s user release
  dir (`fat_disk.rs:98-111, 125-131`) into a 16 MiB FAT16 root.
- **F9** `disk_boot` and `iso_boot` carry explicit x86_32-only guards
  (`tests-integration/tests/disk_boot.rs:14-16`, `iso_boot.rs:8`) and keep them.
- **F10** CI: the two x86_64 build jobs run `cargo xtask build` with **no mtools** installed
  (`.github/workflows/ci.yml:53-54`, `.github/workflows/build-artifacts.yml:27-28`); the x86_32
  build job installs mtools precisely because `create_boot_disk` needs it
  (`ci.yml:81-82`). Both integration jobs already install mtools (`ci.yml:123, 149`).
- **F11** Plan budget: ≤ 15 steps, ≤ 512 bytes (`system/src/boot_plan.rs:11-12`). The x86_32
  plan encodes at 12 steps today; x86_64 will encode the identical token strings, so the budget
  is inherited, not re-derived.
- **F12** `spawn_with_master_disk` (`qemu.rs:64-82`) places a caller-supplied disk at `index=0`
  for the x86_32 boot-disk tests and attaches no master FAT otherwise; `spawn_cdrom`
  (`qemu.rs:132-138`) attaches no disks. The scratch/test disks ride `index=1`
  (`qemu.rs:84-130`).

### Measured facts — **[O]** (this machine, QEMU 10.2.1)

- **O1 [O]** Control: with a data-only MBR (rxpart shape: bootable FAT16 entry, `0x55AA`
  signature, zero boot code) at `if=ide,index=0` and a bootable image on `bus=ide.1` **without**
  a bootindex, SeaBIOS loads the primary-master MBR and hangs — the run had to be killed. A
  data disk at index 0 must never be the BIOS's first boot attempt.
- **O2 [O]** Same rig with `-device ide-hd,drive=…,bus=ide.1,unit=0,bootindex=1` on the
  bootable drive: boots (observed via `isa-debug-exit` status). Adding a third disk at
  `index=1` changes nothing. Note `-drive …,bootindex=N` is rejected by this QEMU
  ("Block format 'raw' does not support the option 'bootindex'") — the bootindex rides the
  `-device`, so the carrier needs the `-drive if=none,id=…` + `-device ide-hd,…` pair.
- **O3 [O]** The real artifact boots on this topology: `target/rosx/release/rosx-x86_64.img` on
  `bus=ide.1,unit=0,bootindex=1` with a foreign data MBR occupying the primary master reaches
  `rose>` on `-debugcon stdio` with the unmodified current kernel. The kernel-booting role of
  the carrier disk does not depend on it being drive 0x80 (consistent with F7).
- **O4 [O]** The current x86_64 stack builds clean on this tree (`cargo xtask build x86_64`).

### Unverified — resolved during execution

- **U1** RESOLVED (M4): every x86_64 integration suite passes root-mounted on the FAT master
  (28/28 green; the only change was retiring a stale arch-fork in `vfs_hdd_flow` — see §6). → §6
- **U2** RESOLVED (M2): `mformat` of the x86_64 64 MiB recipe produced the exact layout
  (`mdir` shows the 31 apps at root, `kernel.elf` + `rom.img` under `/boot`). → §6
- **U3** RESOLVED (M3): no-disk boot logs the mount/service failures and lands on the ROM shell
  with the bootstrap-eight `ls /`. → §6

---

## 2. Design — the topology move

x86_32 leaves the whole primary IDE channel to data because QEMU loads its kernel out-of-band
(`-kernel`). x86_64's kernel arrives *as a disk image*, and today that image occupies primary
master — stealing `ata0d0` from the data volume and polluting the namespace with the
bootloader's own slices.

The fix: **the kernel carrier moves to the secondary channel, data keeps the primary.**

| IDE slot          | x86_32 today                    | x86_64 today                  | x86_64 after                 |
|-------------------|---------------------------------|-------------------------------|------------------------------|
| primary master    | FAT boot disk → `ata0d0` + `ata0d0p0` | kernel carrier → `ata0d0` + `ata0d0p1/p2` (footgun) | **FAT boot disk → `ata0d0` + `ata0d0p0`** |
| primary slave     | persistent HDD → `ata0d1`       | persistent HDD → `ata0d1`     | persistent HDD → `ata0d1`    |
| secondary master  | —                               | —                             | **kernel carrier (BIOS boot only, OS-invisible per F6)** |
| kernel load       | `-kernel` (Multiboot v1)        | BIOS boot, primary master     | BIOS boot, secondary master with `bootindex=1` (O1–O3) |

Result: the guest-visible block namespace on x86_64 becomes byte-identical to x86_32's, the
boot-plan mount token `ata0d0p0` is literally the same string on both arches, and the
`ata0d0p1/p2` footgun disappears because the carrier disk leaves the probed channel.

Concrete QEMU shape for the `DiskImage` medium (proven by O2/O3):

```
-drive if=none,id=rosxboot,format=raw,file=<carrier.img>
-device ide-hd,drive=rosxboot,bus=ide.1,unit=0,bootindex=1
```

Fallback semantics stay x86_32's: with no disk at index 0, the mount step and the disk-sourced
services log-and-continue and the plan's final `exec rom:shell` lands on the ROM shell (U3).

---

## 3. Milestones

### M1 — x86-runner: the carrier moves to the secondary bus
**Files:** `arch/x86-runner/src/lib.rs`, `arch/x86-runner/src/main.rs`

- `qemu_command`'s `DiskImage` arm emits the two-part attach above instead of the bare
  `-drive format=raw,file=…` (F5), freeing the primary master for the data volume.
- Add `pub fn boot_disk_image(kernel_elf: &Path) -> PathBuf` deriving the workspace FAT boot
  disk from the kernel path (`…/target/<dir>/<profile>/<bin>` → `target/<dir>-boot-disk.img`,
  the F4 formula) and `pub fn attach_fat_boot_disk(command, image)` emitting
  `if=ide,index=0,media=disk,format=raw` — but only when the image exists, with a `note:` line
  otherwise, mirroring `attach_persistent_hdd`'s politeness.
- `main.rs` (the `cargo run -p rosx` path) calls `attach_fat_boot_disk` before
  `attach_persistent_hdd`, so `cargo run` gets the disk flow whenever `cargo xtask build` has
  produced the image, and degrades to the ROM-shell plan when it has not.
- Unit tests: update the two `DiskImage` arg expectations (interactive/headless) for the new
  pair and the `bootindex`; add tests for `boot_disk_image` path derivation and the
  attach-only-when-present rule.
- **Done when:** `cargo test -p x86-runner` green; a manual QEMU launch of the *current*
  carrier in the new shape reaches `rose>` (O3 re-proven against the final runner code, not
  the hand-typed experiment).

### M2 — xtask: FAT boot disk on, full ROM off
**Files:** `xtask/src/arch.rs`, `xtask/src/main.rs`

- `X86_64.fat_boot_disk: false → true`; `X86_64.rom_apps: &USER_APPS → &BOOTSTRAP_APPS` (F3).
- `launch()` (F5): attach `boot_disk::boot_disk_path(spec)` at `index=0` when
  `spec.fat_boot_disk` — one flag-driven rule for both arches, replacing the
  `MultibootKernel`-only check.
- `cargo xtask build x86_64` then writes `target/rosx-boot-disk.img` through the untouched
  `boot_disk.rs` recipe: MBR + FAT16 + all 31 ELFs at root + `/boot/kernel.elf` +
  `/boot/rom.img` (the ROM at this point is already the slim eight, matching the kernel's
  embedded copy).
- **Done when:** the image exists after a build; `mdir` shows the 31 apps at root and both boot
  files under `/boot` (**U2**); the 8.3-name gate (`boot_disk.rs:56-67`) stays silent (every
  `USER_APPS` name already fits).

### M3 — the x86_64 boot plan mirrors x86_32
**Files:** `arch/x86_64/src/main.rs`

- Replace the all-ROM plan (F1) with F2's twelve steps, token for token, using the same
  protocol-crate probe constants (`vfs_protocol::wire::SERVICE_NAME`,
  `block_protocol::manager::wire::SERVICE_NAME`, …) already imported there.
- No other `main.rs` change: ROM embed path, init ELF embed, kstatsd, memory map stay.
- **Done when:** headless boot reaches `rose>` with `[INIT]` lines showing the mount of
  `ata0d0p0` succeeding and `/randomd` `/echo` `/pong` coming up **from the disk** (watch the
  `[INIT]` diagnostics); `ls /` lists the disk's ELFs; `attach` lists `ata0d0` (+`ata0d0p0`)
  and `ata0d1` and **no** `ata0d0p1/p2`; and the no-disk boot (plain `cargo run` with the boot
  disk moved away) falls through to the ROM shell (**U3**).

### M4 — the integration harness roots x86_64 on disk too
**Files:** `tests-integration/src/qemu.rs` (+ per-test fixes discovered by the runs)

- `attach_multiboot_master_disk` becomes unconditional for both boot media (F8): every
  `QemuSession` except `spawn_cdrom` gets the full-apps FAT master at `index=0` (free on x86_64
  since M1). Rename to match (`attach_fat_master_disk`).
- `spawn_with_master_disk` keeps its caller-supplied index-0 disk untouched (F12).
- Run the full x86_64 suite (`cargo test -p tests-integration -- --test-threads=1`) and the
  full x86_32 suite (**regression check — no x86_32 file changes except none**, so x86_32 must
  stay green by construction).
- Expected touchpoints where x86_64 ROM-root assumptions may surface (audit list, fix what the
  runs actually break): `ipc_vfs` (x86_64-only suite), `boot_and_shell`, `vfs_hdd_flow`,
  `partitioned_hdd_flow`, `fat_partition_flow`, `expose_corrupt_hdd`, `ipc_bulk` (launches
  `bulk` by name — now a FAT-root lookup), `elf_arguments`, `shell_command_arguments`. The
  x86_32 runs of these same suites under a root-mounted FAT master are the behavioral
  reference.
- **Done when:** both arches' full integration suites are green (**U1**).

### M5 — CI + docs
**Files:** `.github/workflows/ci.yml`, `.github/workflows/build-artifacts.yml`, `AGENTS.md`

- Add the mtools install step to both **Build Kernel (x86_64)** jobs (F10) — `create_boot_disk`
  now runs there too.
- Extend the x86_64 artifact uploads with `target/rosx-boot-disk.img`.
- AGENTS.md rewrite of the affected passages: the x86_64 build/run section (new boot disk,
  secondary-carrier topology, slim ROM, x86_32-shaped plan), the `cargo run` flow note, the
  `ata0d0p1/p2` footgun paragraph (retired on x86_64 — replaced by the new topology note), the
  boot-plan section (both ports speak the same plan; the "bare name is implicit-ROM
  transitional form" sentence retires with x86_64's adoption), and the ROM roster paragraph
  (`rom_apps` roster is now `BOOTSTRAP_APPS` on both ports).
- **Done when:** CI is green on the PR and AGENTS.md describes the tree as it is.

---

## 4. Verification ledger (every claim ends green or annotated)

| # | Check | Proves |
|---|-------|--------|
| V1 | `cargo test -p x86-runner` | M1 unit surface |
| V2 | `cargo xtask test --skip-integration` | workspace unit floor |
| V3 | `cargo xtask build x86_64` → `mdir -i …@@32256 ::/` and `::/boot` | U2 (FAT layout, /boot staging) |
| V4 | headless boot via the new runner: `rose>`, `[INIT]` mount line, disk-sourced services | M1+M3 end-to-end |
| V5 | shell `attach` → `ata0d0`+`ata0d0p0`, `ata0d1`; no `ata0d0p1/p2` | the namespace parity goal |
| V6 | shell `ls /` lists app ELFs; `ps`/`mem` respond; `snake` (or `pi`) launches from disk | disk-as-filesystem parity |
| V7 | no-disk boot (`index=0` absent) → `[INIT]` failure logs, ROM shell | U3 fallback |
| V8 | `cargo test -p tests-integration -- --test-threads=1` (x86_64) | U1 |
| V9 | `ROSX_ARCH=x86_32 cargo test -p tests-integration -- --test-threads=1` | no x86_32 regression |
| V10 | `cargo xtask build x86_32` unchanged green | M2 touched shared code only via flags |

---

## 5. Risks & mitigations

- **SeaBIOS picks the wrong disk.** O1 shows the failure mode (hang) explicitly; O2/O3 show
  `bootindex` on the carrier device wins deterministically. The FAT disk's MBR keeps its
  bootable byte (x86_32 parity with GRUB-era media) and is never attempted first.
- **x86_64 tests lean on ROM-root assumptions** (ROM volume = `/`, full ROM roster). Mitigated
  by the M4 audit list and by x86_32 already running every one of these suites on a root-mounted
  FAT master. The ROM *program source* (`rom:` tokens, `RomRead`) is untouched — only VFS
  namespace visibility of ROM files changes, by root shadowing, exactly as x86_32 does.
- **Stale boot disk under `cargo run`.** `cargo run -p rosx` does not rebuild the disk image; a
  stale `target/rosx-boot-disk.img` mounts stale ELFs. Mitigation: attach-if-present (M1),
  documented in AGENTS.md (`cargo xtask run` rebuilds; `cargo run` reuses, same as today's
  kernel-embed asymmetry).
- **`bootindex` with `-drive`** is unsupported (O2) — the design uses the `if=none` + `-device`
  pair, which is the documented-supported form and what O2/O3 measured.
- **The carrier becoming OS-invisible** (F6) is a feature for the namespace but removes any
  diagnostic path to the carrier from inside the guest. Accepted: nobody reads the carrier from
  the guest today except through the footgun slices this plan removes.

---

## 6. Execution ledger (2026-11-09, branch `x86_64-boot-from-disk`, QEMU 10.2.1)

Commits, in milestone order:

| Commit | Milestone | Change |
|--------|-----------|--------|
| `42b2086` | M1 | Carrier on `bus=ide.1,unit=0,bootindex=1`; `boot_disk_image` + `attach_fat_boot_disk`; runner `main.rs` wires it; 4 new unit tests (17 green) |
| `d073ba2` | M2 | x86_64 `fat_boot_disk: true`, `rom_apps: &BOOTSTRAP_APPS`; `launch()` attaches at `index=0` on the flag |
| `d50f1a7` | M3 | x86_64 boot plan mirrors x86_32 token-for-token |
| `3d985b3` | M4 | Harness attaches the full-apps FAT master for both media (`attach_fat_master_disk`); retires a stale `vfs_hdd_flow` arch-fork |
| `2d7bca6` | M5 | CI mtools + `target/rosx-boot-disk.img` upload; AGENTS.md; `debug-qemu.sh` aligned |

Verification results (the §4 ledger):

- **V1** `cargo test -p x86-runner` → 17 passed, 0 failed.
- **V2** `cargo test` on the shared crates (kernel, rxfs, rxfat, rxpart, fs-core, x86-runner) →
  402 + 121 + … passed, 0 failed.
- **V3** `cargo xtask build x86_64` → `target/rosx-boot-disk.img`; `mdir ::/` lists the 31 app
  ELFs, `mdir ::/boot` lists `kernel.elf` + `rom.img` (**U2**).
- **V4** Headless boot: `[INIT] boot plan of 12 steps` → atad/blockd up, `mount ok
  device=ata0d0p0 dir=/`, `/randomd` `/echo` `/pong` `[INIT] … ready` from disk, `rose>`.
- **V5** Shell `attach` → `ata0d0p0` (type 06, clean) + `ata0d1`; **no** `ata0d0p1/p2` — the
  footgun is gone.
- **V6** `ls /` lists every app ELF; `ls /boot` → `kernel.elf  rom.img`; the disk shell runs.
- **V7** No-disk boot → falls through to the ROM shell; `ls /` shows the bootstrap eight
  (**U3**).
- **V8** `cargo test -p tests-integration -- --test-threads=1` (x86_64) → all suites green after
  M4 (**U1**). One test, `vfs_hdd_flow`, carried an x86_64-only branch asserting an *empty*
  namespace before registration (the old all-ROM plan's world); its x86_32 branch already
  documented the unified truth ("the boot plan registers both drives before the shell"), so the
  fork collapsed to the x86_32 behavior. Green after the change.
- **V9** `ROSX_ARCH=x86_32 cargo test -p tests-integration -- --test-threads=1` → 30 `test
  result: ok`, exit 0 — no x86_32 regression.
- **V10** `cargo xtask build x86_32` → clean, writes its own FAT boot disk.

Not covered by the original milestones but changed for consistency (both in `2d7bca6`):

- `arch/x86_64/debug-qemu.sh` (and the `start-qemu-background.sh` / `ensure-qemu-running.sh`
  family) launched the old primary-master, no-FAT-disk topology; updated to the secondary-bus
  carrier + attach-if-present FAT disk so debugging matches the shipping launch.
- Both CI **Build Kernel (x86_64)** jobs gained the mtools install (`create_boot_disk` now runs
  there) and upload `target/rosx-boot-disk.img` alongside the carrier image.

