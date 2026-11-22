# Platform Runner Abstraction — One Trait, One Implementation per Arch Family

> **Status:** proposed, not started. Written 2026-11-21.
> **Branch base:** `main` at `54034e0` (#186). Every milestone applies to this base directly.
> **Feature:** *how a built kernel becomes something QEMU boots, and how QEMU is launched,* stops
> being an x86-only API plus three duplicated tables. A host-side `runner-core` crate defines a
> `PlatformRunner` trait, a `SessionSpec` describing what a session needs, and one shared table of
> per-arch build facts; `arch/x86-runner` becomes the x86 implementation; a small registry crate
> selects a runner by name for `xtask` and the integration harness. A third family (the RISC-V
> port) then adds one crate that implements the trait instead of editing four call sites.
> **Builds on:** `plans/x86-runner-unification.md` (executed 2026-11-01). That plan kept the
> *mechanisms* per arch and the API mechanism-agnostic (its §3.3, §6) and deferred "a single merged
> Arch table ... until a third arch appears" (its §6). The third arch is now planned; this plan
> executes the deferral.
> **Independent of:** `plans/harness-console-channels.md`. Neither plan needs the other; §7 names
> the one shared file.
> **Method:** every repo fact below was verified against this working tree (paths cited).
> Commands this plan cannot prove green are marked **`unverified`**.
> 
> **Fact refresh (2026-11-22):** the carrier swap (`plans/x86_64-carrier-swap.md`) retired the
> `bootloader`-crate carrier and with it `BootMedium`, `prepare_run_medium`,
> `prepare_run_medium_in`, `create_bios_disk_image*` and `attach_fat_boot_disk`;
> `x86_runner::run_medium(kernel, arch)` now derives the unified boot-disk path for **both**
> arches (the `<kernel_dir_name>-boot-disk.img` rule the §2.3 complaint counted three times
> now lives in exactly two places) and `qemu_command` is a single boot-disk arm. The plan
> stands — on a simpler base.

---

## 1. Goal & Non-Goals

### Goal
- One definition of "which platform is this", replacing three overlapping ones (§2.1), so that:
  - build facts (kernel manifest, target specs, output directory and binary names) have one
    source of truth used by `xtask` and the harness;
  - a platform's QEMU binary, machine flags, boot-medium preparation, device attachment and
    flashable-image creation live behind one trait with one implementation per arch family,
    living in that family's `arch/` area;
  - sessions are described by *what they need* (boot disk, data disk, NIC, optical boot) and the
    runner decides *how* (IDE indexes and `ne2k_*` flags stop leaking into the harness, §2.3);
  - tests that exist only because of an x86 device say so, and fail with a clear message on a
    platform that lacks it.

### Non-Goals
- **No behaviour change on x86.** The QEMU command lines, image bytes, artifact paths and
  environment variables are unchanged; §5 M1 pins them before anything moves.
- **No `riscv-runner` crate yet.** It would have no production caller and AGENTS.md forbids
  production API without one. §3.7 proves the trait fits RISC-V on paper; the port plan adds the
  crate with its first caller.
- **No boot-protocol change** (the unification plan's Non-Goals stand: BiosBoot for x86_64,
  Multiboot `-kernel` for x86_32).
- **No feature-gating of the x86 runner out of `xtask` or the harness.** Without a second
  implementation there is nothing to verify the gating against (§6).
- **`xtask`'s boot-test is untouched**: it tests the x86 bootloader stages and keeps its own
  scenario table (§2.5).
- **No kernel, `system`, protocol, app or ABI change.**

## 2. Verified baseline facts (2026-11-21, paths)

1. **Three overlapping "arch" definitions.**
   - `x86_runner::Arch` (`arch/x86-runner/src/lib.rs:6`): `name`, `from_name`, `qemu_binary`.
   - `tests_integration::arch::Arch` (`tests-integration/src/arch.rs`): `kernel_manifest`,
     `kernel_target_spec`, `kernel_dir_name`, `kernel_binary`, `user_dir_name`, plus `boot()`
     back to the runner's enum.
   - `xtask::arch::ArchSpec` (`xtask/src/arch.rs:9`): `name`, `kernel_package`,
     `kernel_target_spec`, `user_target_spec`, `kernel_dir_name`, `kernel_binary`,
     `user_dir_name`, `boot`, `fat_boot_disk`, `rom_apps`.
   Four facts (`kernel_target_spec`, `kernel_dir_name`, `kernel_binary`, `user_dir_name`) appear
   in both the harness and xtask tables. The unification plan called the build and harness tables
   "legitimately different" (its §3.4); today they overlap almost entirely.
2. **The runner's public surface** (`arch/x86-runner/src/lib.rs`): `Arch`, `BootMedium`
   (`DiskImage`, `MultibootKernel`, `CdImage`), `LaunchMode`, `ImageError`, `GrubTooling`,
   `run_medium`, `prepare_run_medium`, `prepare_run_medium_in`, `create_bios_disk_image`,
   `create_grub_image`, `qemu_command`, `attach_ne2k_isa`, `attach_ne2k_pci`,
   `persistent_hdd_image`, `boot_disk_image`, `attach_fat_boot_disk`, `attach_persistent_hdd`;
   plus a binary (`src/main.rs`, argv `<kernel> <arch> [--no-run]`) that `arch/*/run.sh` invokes.
3. **Device topology leaks into the callers.** The IDE strings
   `if=ide,index=0,media=disk,format=raw,file=…` (boot disk) and `…index=1…` (data disk) are
   written out in `x86-runner` (`attach_fat_boot_disk`, `hdd_drive_arg`), in `xtask`'s `launch`
   (`xtask/src/main.rs:84-92`) and in the harness (`tests-integration/src/qemu.rs`:
   `attach_fat_master_disk`, `spawn_with_master_disk`, `spawn_with_hdd_disk`,
   `spawn_with_scratch`). The boot-disk path derivation exists twice more
   (`x86_runner::boot_disk_image`, `xtask/src/boot_disk.rs::boot_disk_path`).
4. **Nine session constructors, six near-duplicates.** `QemuSession::spawn_*`
   (`qemu.rs:37-170`) repeat `prepare_run_medium_in` → `qemu_command(Headless)` →
   `attach_fat_master_disk` → device flags → `spawn_capturing`; they differ only in which devices
   follow.
5. **A fifth copy of QEMU knowledge**: `xtask/src/boot_test.rs` carries its own table of
   binaries, CPUs and RAM per scenario (`BootMachine`, lines 17-61) and the `isa-debug-exit`
   device. It exercises the x86 bootloader stages, not the platform runner. Out of scope.
6. **xtask arch-conditionals.** `check-isa` and `era-image` refuse non-x86_32 specs by comparing
   `spec.boot` (`xtask/src/main.rs:377`, `:546`). They are genuinely x86_32-specific commands;
   they keep an explicit name comparison.
7. **Dependency arrows** (`xtask/arch-rules.txt:59,68,69`): `tests-integration: input-protocol
   libc rxpart tempfile x86-runner`, `x86-runner: anyhow bootloader tempfile`, `xtask: rxpart
   system x86-runner`. `check-arch` enforces them; new crates need lines there.
8. **Harness selection**: `ROSX_ARCH` unset or `x86_64` → x86_64; `x86_32` → x86_32; anything
   else panics `ROSX_ARCH is set to '<v>'; valid values are: x86_64, x86_32`
   (`tests-integration/src/arch.rs`, with unit tests including the panic text).
9. **Test requirements are implicit.** `ata_*`, the `*_hdd_flow`/partition tests, `ne2k_*`,
   `arp_*`, `ip_*` and `iso_boot` depend on x86 devices; only `ipc_vfs.rs` calls the
   `not_ported_to_x86_32()` skip, which is about a *port subset*, not a missing device.
10. **`cargo xtask check` runs `cargo test --workspace`** excluding only `tests-integration`
    (`xtask/src/main.rs:247-250`), so unit tests added in `runner-core`, `runner-registry` and
    `x86-runner` are in the gate; harness-crate unit tests are not (the sibling plan changes
    that). QEMU suites need `cargo xtask test`.
11. **Image artifacts that must not move**: `<kernel>/rosx-x86_64.img` next to the kernel ELF
    (CI upload and `arch/x86_64/DEBUG_SETUP.md:147`), the x86_32 ISO `rosx-x86_32.img`, and
    `target/rosx-boot-disk.img` / `target/rosx-i486-boot-disk.img`.
12. **Repo rules this plan honours** (AGENTS.md): no comments; no test-only production surface
    (every trait method, type and crate added here gets a non-test caller in the same commit);
    `kernel/` untouched; `xtask/arch-rules.txt` updated in the commit that adds an arrow; commit
    messages carry no prefix; each step ends with the `retro` skill's step-end report.

## 3. Design decisions

### 3.0 Decisions taken (from review, 2026-11-21)

| # | Decision | Source |
|---|---|---|
| D1 | A new arch-neutral core crate holds the abstraction | maintainer |
| D2 | `arch/x86-runner` is kept and adapted as the x86 implementation, not renamed | maintainer |
| D3 | Implementations live per arch family under `arch/`; the RISC-V runner arrives with the RISC-V port | maintainer |
| D4 | A registry crate selects a runner by name (the core cannot depend on implementations) | maintainer (accepted 2026-11-21) |
| D5 | `BootMedium::MultibootKernel` becomes `DirectKernel` (QEMU `-kernel`), the mechanism-neutral name | maintainer (accepted 2026-11-21) |
| D6 | Sessions are described by an abstract `SessionSpec`; capabilities are a small fixed enum | maintainer (accepted 2026-11-21) |

### 3.1 Crates

| Crate | Role | Depends on |
|---|---|---|
| `arch/runner-core` (new, host lib) | `PlatformRunner`, `PlatformFacts`, `BootMedium`, `LaunchMode`, `ImagePlacement`, `SessionSpec`, `Nic`, `Capability`, `ImageError`, `UnsupportedRequest` | none |
| `arch/x86-runner` (kept) | `X86Runner` for x86_64 and x86_32; keeps the `bootloader`/GRUB/IDE/ne2k knowledge; keeps its binary | `runner-core`, `anyhow`, `bootloader`, `tempfile` |
| `arch/runner-registry` (new, host lib) | `find(name)`, `all()` over the known runners | `runner-core`, `x86-runner` |

`xtask` and `tests-integration` depend on `runner-core` and `runner-registry`; neither names
`x86-runner` any more. Placement follows the existing rule that platform code goes in
`arch/<platform>/` and x86-family code shared across widths is named for the family.

### 3.2 `PlatformFacts` — the one table
`name`, `kernel_package`, `kernel_manifest`, `kernel_target_spec`, `user_target_spec`,
`kernel_dir_name`, `kernel_binary`, `user_dir_name`, plus a derived `boot_disk_path(target_dir)`
(the `<kernel_dir_name>-boot-disk.img` rule that exists three times today, §2.3). Pure data: no
paths are touched. `xtask::ArchSpec` keeps what is genuinely xtask's — `rom_apps`,
`fat_boot_disk` — and holds a runner reference instead of the duplicated fields.

### 3.3 The trait

```
trait PlatformRunner: Sync
    facts(&self) -> &PlatformFacts
    supports(&self, Capability) -> bool
    boot_medium(&self, kernel_elf: &Path, placement: ImagePlacement) -> Result<BootMedium, ImageError>
    qemu_command(&self, medium: &BootMedium, spec: &SessionSpec) -> Result<Command, UnsupportedRequest>
    build_image(&self, kernel_elf: &Path) -> Result<PathBuf, ImageError>      // `cargo xtask image`

enum ImagePlacement { BesideKernel, In(PathBuf) }     // today's prepare_run_medium vs _in
enum BootMedium { DirectKernel(PathBuf), DiskImage(PathBuf), CdImage(PathBuf) }
```

- `boot_medium` and `build_image` absorb `run_medium`/`prepare_run_medium(_in)` and the
  `create_bios_disk_image` / `create_grub_image` split in `xtask`'s `create_flashable_image`
  (`xtask/src/main.rs:210-227`): x86_64 returns the BiosBoot image, x86_32 the GRUB ISO. The
  GRUB missing-tools hint survives because `ImageError` wraps any `Display` error.
- `qemu_command` returns a builder the caller may still extend with *session-control* knobs
  (the harness's monitor socket, piped stdio); it never knows them.

### 3.4 `SessionSpec` — what a session needs

```
struct SessionSpec {
    mode: LaunchMode,                 // Interactive | Headless, as today
    boot_disk: Option<PathBuf>,       // the FAT master / boot disk image
    data_disks: Vec<PathBuf>,         // 0 or 1 today (persistent hdd, scratch, partitioned image)
    nic: Option<Nic>,                 // Ne2kIsa { multicast } | Ne2kPci
}
```
- x86 maps `boot_disk` to IDE index 0 and the first data disk to IDE index 1 — the topology that
  `atad`'s port pool and the boot plan (`ata0d0`, `ata0d1`) assume; more than one data disk is
  `UnsupportedRequest`. A CD-ROM boot stays a `BootMedium::CdImage`, not a spec field.
- The harness keeps *creating* the FAT master (`fat_disk::create_boot_disk`) and any carved
  images; it passes paths. The runner only attaches them.
- The persistent host disk (`$HOME/tmp/rosx-hdd.img`, `ensure_hdd_image`) stays in the x86
  runner as the interactive default that `xtask run` and the run binary put into `data_disks`.

### 3.5 `Capability`
`AtaDisks`, `Ne2kIsaNic`, `Ne2kPciNic`, `CdromBoot` — exactly the device families the existing
x86-only tests depend on (§2.9), each with real callers (the tests that gate on them). A request
the runner cannot honour fails in `qemu_command` with `UnsupportedRequest`, naming the
capability, so a platform lacking a device gets a loud, readable error instead of a QEMU
argument failure. The *standard session's composition* on a disk-less platform (ROM-only boot,
RAM-backed root) is the port plan's decision, not this plan's.

### 3.6 Registry and selection
`runner-registry::find(name) -> Option<&'static dyn PlatformRunner>` and `all()`.
`xtask::arch::find`/`ALL` and the harness's `ROSX_ARCH` parsing both delegate to it; the harness
keeps its behaviour (unset or empty selects x86_64; an unknown value panics with a message that
lists the registry's names). The harness's `Arch` enum and its `boot()` back-reference are
deleted.

### 3.7 Paper check: does the trait fit RISC-V?
Written before building so the trait is not accidentally x86-shaped. Not code; no crate exists.

| Trait member | x86_64 / x86_32 (today) | riscv64 (sketch, `qemu-system-riscv64`) |
|---|---|---|
| `facts` | names, JSONs, dirs | `rosx-riscv64` names; same fields |
| `boot_medium` | BiosBoot image / the kernel ELF | `DirectKernel(kernel ELF)` via `-kernel`, firmware from `-bios default` |
| `qemu_command` base | `-cpu 486` (x32), `-debugcon stdio -no-reboot`, `-display none` headless | `-machine virt -cpu rv64 -m <ram> -smp 1 -bios default -no-reboot -display none` |
| `boot_disk` | IDE index 0 | `UnsupportedRequest` until a disk daemon exists; later a virtio-blk device |
| `data_disks` | IDE index 1 | likewise |
| `nic` | `ne2k_isa` / `ne2k_pci` | `UnsupportedRequest` |
| `supports` | all four true | all four false |
| `build_image` | BIOS image / GRUB ISO | the kernel ELF itself |
| exit device for boot-test | `isa-debug-exit` (xtask-only) | not in scope |

Nothing in the trait mentions IDE, PCI, debugcon or Multiboot.

## 4. What moves, what stays

| Piece | Verdict |
|---|---|
| `x86_runner::{Arch, run_medium, prepare_run_medium, prepare_run_medium_in, qemu_command, attach_*}` | **fold into** `X86Runner`'s trait methods; the free functions are deleted once their callers migrate (M4) |
| `x86_runner::{create_bios_disk_image, create_grub_image}` | become private helpers behind `build_image`/`boot_medium` |
| `x86_runner::{persistent_hdd_image, ensure_hdd_image, boot_disk_image}` | the first two stay as the x86 interactive default; the path derivation moves to `PlatformFacts::boot_disk_path` |
| `x86-runner` binary + `arch/*/run.sh` argv contract (`<kernel> <arch> [--no-run]`) | **stay**; the binary builds a `SessionSpec` and calls the trait |
| `xtask::arch::ArchSpec` | keeps `rom_apps`, `fat_boot_disk`; loses the duplicated facts and `boot` |
| `xtask/src/boot_disk.rs::boot_disk_path` | delegates to `PlatformFacts::boot_disk_path` |
| `tests_integration::arch::Arch` | **deleted**; selection via the registry |
| `QemuSession::spawn_*` (nine constructors) | **stay** with the same names and signatures; internally one spec-driven path |
| `fat_disk::create_boot_disk`, carved-image helpers | **stay** in the harness |
| `xtask/src/boot_test.rs` | **untouched** (§2.5) |
| Kernel boot code, images, CI workflows, artifact names | **untouched** (§2.11) |

## 5. Milestones (each one commit; approve-and-go per step)

Every milestone ends with the `retro` skill's step-end report in the commit report. The friction
log lives in `plans/platform-runner-abstraction-impl.md`, created when M1 starts.

### M1 — Pin today's x86 command lines (zero behaviour change)
- `x86-runner` unit tests asserting the **complete** argument vector of `qemu_command` for every
  (arch × medium × mode) combination in use — x86_64 disk image, x86_32 `-kernel`, CD image,
  Interactive and Headless — and the exact flags of `attach_ne2k_isa`, `attach_ne2k_pci`,
  `attach_fat_boot_disk` and `hdd_drive_arg`. The existing tests (`lib.rs:377-536`) assert
  fragments; these pin everything so any later drift is a red test, not a QEMU surprise.
- **Gate:** `cargo xtask check` (the crate's unit tests are in the gate, §2.10).
- **Commit:** `Pin the complete x86 QEMU argument vectors before refactoring the runner`

### M2 — One table of build facts
- New `arch/runner-core` with `PlatformFacts` (and `boot_disk_path`); workspace member; a
  `runner-core:` line in `xtask/arch-rules.txt`.
- `x86-runner` exposes the facts for its two ports; `xtask::ArchSpec` and the harness `Arch`
  take their four overlapping fields (and the manifest/package names) from them. Unit tests:
  the two ports' facts equal the strings the tables carry today (pinned from §2.1).
- **Gate:** `cargo xtask check`; `cargo xtask build` and `cargo xtask build x86_32` produce the
  same artifact paths (§2.11); `cargo xtask test` for both arches.
- **Commit:** `Share one table of per-arch build facts between xtask and the harness`

### M3 — The trait; xtask and the run binary move onto it
- `runner-core` gains `PlatformRunner`, `BootMedium` (with `DirectKernel`), `ImagePlacement`,
  `SessionSpec`, `Nic`, `Capability`, `UnsupportedRequest`, `ImageError`, `LaunchMode`.
- `x86-runner` implements it as `X86Runner` (one struct, two statics); `arch/runner-registry`
  is added with `find`/`all`; arch-rules lines updated (§3.1).
- `xtask` (`prepare_medium`, `launch`, `create_flashable_image`) and the `x86-runner` binary use
  the trait. The free functions stay for now: the harness still calls them.
- Unit tests: spec→arguments for each x86 mapping (boot disk at index 0, data disk at index 1,
  both NIC models, interactive default), rejection of a second data disk, and **the trait path
  reproduces M1's pinned vectors byte-for-byte**.
- **Gate:** `cargo xtask check`; `cargo xtask build`; `cargo run --manifest-path
  arch/x86-runner/Cargo.toml -- <kernel> x86_64 --no-run` produces an image **byte-identical** to
  a pre-change build (baseline captured before M3 — **`unverified`** until the executor does
  so); `cargo xtask image` and `cargo xtask image x86_32`; manual `cargo xtask run [arch]` to the
  prompt (**`unverified`**).
- **Commit:** `Introduce PlatformRunner and move xtask and the run binary onto it`

### M4 — The harness builds sessions from a `SessionSpec`
- `tests-integration`: the nine `spawn_*` constructors build a `SessionSpec` and go through one
  private path (prepare medium → `qemu_command` → session-control knobs → spawn); the six
  duplicated attach sequences disappear; `Arch` is replaced by the registry selection with the
  same `ROSX_ARCH` semantics. The x86 free functions and `x86_runner::Arch` are deleted (their
  last callers are gone); `tests-integration` and `xtask` stop depending on `x86-runner`
  directly (arch-rules updated).
- Unit tests: `ROSX_ARCH` selection (unset, empty, valid names, the unknown-value panic text —
  the existing tests carry over).
- **Gate:** `cargo xtask check`; full `cargo xtask test` (x86_64) and
  `ROSX_ARCH=x86_32 cargo test -p tests-integration -- --test-threads=1` — the true oracle: any
  flag drift fails loudly; `cargo xtask decisions` unchanged.
- **Commit:** `Build integration sessions from a platform-neutral SessionSpec`

### M5 — Name each test's platform requirements
- `tests_integration` gains `supports(Capability)`; the test files whose subject is an x86
  device (`ata_*`, the hdd/partition/corrupt-table flows, `ne2k_*`, `arp_*`, `ip_*`, `iso_boot`)
  start with a one-line guard that skips with a stated reason when the platform lacks the
  capability. A spec the runner cannot honour already panics with the capability named
  (§3.5); the guards make the skip explicit and polite. `not_ported_to_x86_32()` is left alone —
  it expresses a port subset, not a device.
- **Gate:** `cargo xtask check`; both arches' integration suites (the guards are always true on
  x86, so the suites are unchanged).
- **Commit:** `Declare which platform capability each device-specific integration test needs`

### M6 — Documentation sweep
- `docs/architecture/boot-media-and-harness.md`: a "Platform runners" paragraph (the trait, the
  facts table, the registry, where an implementation lives) and an invariant line.
  `AGENTS.md` keeps its "No bootimage" line (still true) unless a fact it states has changed.
  Verify with `grep -rn "x86_runner::\|tests_integration::arch" docs AGENTS.md`.
- `docs/todo/` entry for the deferred items in §6 with their triggers.
- **Commit:** folds into whichever commit last changes the fact, per repo habit.

## 6. Deferred, recorded not dropped
- **`arch/riscv-runner`** — added by the RISC-V port plan with its first caller; §3.7 is the
  fit check it starts from.
- **Feature-gating `x86-runner` (and its `bootloader` dependency, a heavy build) out of builds
  that do not need it** — only meaningful once a second implementation exists to test against.
- **`xtask` boot-test machine table** — could draw its binary names from the runner; trivial,
  but it belongs to the bootloader test, not the platform runner.
- **A `run.sh`/runner binary per platform** — the RISC-V port decides whether `cargo run` is
  wired for it or whether `cargo xtask run riscv64` is the only entry point.
- **Standard-session composition on disk-less platforms** (ROM-only, RAM-backed root) — the
  RISC-V port plan's decision.

## 7. Risks

| # | Risk | Mitigation |
|---|------|------------|
| 1 | QEMU flag drift under the 38 test files | M1 pins complete argument vectors before any move; M3 asserts the trait path reproduces them; M4 gates on both arches' full suites |
| 2 | The trait is secretly x86-shaped | §3.7 paper check against RISC-V before building; no member mentions IDE, PCI, debugcon or Multiboot; `SessionSpec` is abstract |
| 3 | Over-modelling (`Capability`, registry) with only one implementation | Each item has a production caller in the commit that adds it; the enum is limited to the four device families existing tests already depend on; the registry is justified by two consumers needing one selection path |
| 4 | Image bytes or artifact paths change | M3 byte-compares the BiosBoot image against a pre-change build; §2.11 paths asserted in M2 and M3 |
| 5 | `cargo run` (nested cargo through `run.sh`) breaks | The binary keeps its argv contract; M3's gate runs it with `--no-run`; manual interactive boot is flagged **`unverified`** |
| 6 | `tests-integration/src/qemu.rs` conflicts with the console-channels plan | The only shared file. This plan edits the constructors and command assembly; the other edits console capture, key injection and pacing. Whichever lands second rebases; neither plan's gates depend on the other |
| 7 | Large diffs in `xtask/src/main.rs` | The change is mechanical call-site substitution, split across M2 (facts), M3 (trait) and M4 (harness) so no commit mixes concerns |

## 8. Verification matrix

| Command (cwd) | Proves |
|---|---|
| `cargo xtask check` (root) | the gate: arrows in `arch-rules.txt`, style, unit tests of `runner-core`, `runner-registry`, `x86-runner` |
| `cargo xtask build [x86_32]` (root) | both build chains and artifact paths through the shared facts |
| `cargo xtask image [x86_32]` (root) | `build_image` on both arches (GRUB needs `grub-mkrescue`, `xorriso`, mtools — **`unverified`** where missing) |
| `cargo run --manifest-path arch/x86-runner/Cargo.toml -- <kernel> x86_64 --no-run` (root) | the binary path and byte-identical image |
| `cargo xtask test` (root) | x86_64 QEMU suite through `SessionSpec` (M4-M5) |
| `ROSX_ARCH=x86_32 cargo test -p tests-integration -- --test-threads=1` (root, `ROSX_KERNEL_ELF` set) | x86_32 QEMU suite |
| `cargo xtask verify` (root) | merge evidence for the PR series (~40 min) — **`unverified`** until run at the final head |

The authoring environment has no QEMU, mtools or GRUB, so the rows that need them are
**`unverified`** until run by the executor.

## 9. Interfaces summary
After M6: `runner-core` exports the trait and its types; `x86-runner` exports `X86Runner` (and
keeps its binary, argv `<kernel> <arch> [--no-run]`); `runner-registry` exports `find` and `all`.
`xtask`'s user-facing commands, the `ROSX_*` environment variables (`ROSX_ARCH`,
`ROSX_KERNEL_ELF`, `ROSX_QEMU_PROFILE`), every test file's behaviour on x86, all artifact names
and paths, the kernel, `system`, the protocols and the apps are unchanged. No ABI or wire change
anywhere in this plan.
