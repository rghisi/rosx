# Unified x86 Runner — Standardizing Run & Image Creation

> **Status:** Executed 2026-11-01 (M1–M5). AMENDED during execution at
> §2.10/§3.2 (member topology, `prepare_run_medium` added to the API) and
> §8 (results recorded there). Written 2026-11-01.
> **Branch base:** `origin/main` at `62c530d` (#98). All milestones apply to
> this base directly.
> **Feature:** one `arch/x86-runner` crate that owns *how a built kernel is
> turned into something QEMU boots and how QEMU is launched*, for both x86
> ports. The mechanisms stay per-arch (BiosBoot disk image vs Multiboot
> `-kernel` — see §1 Non-Goals why they cannot merge yet); the *interface*
> and the *call sites* become one.
> **Method:** every repo fact below was verified against this working tree
> (paths cited). Commands this plan cannot prove green are marked
> **`unverified`**.

---

## 1. Goal & Non-Goals

### Goal
- Today boot/run knowledge exists in **four** divergent implementations
  (§2). Converge them on one host-side crate so that:
  - the QEMU flag set has one source of truth;
  - `cargo xtask run`/`build`/`image` behave symmetrically on both arches;
  - `tests-integration` boots through the same code path as developers;
  - a future arch (or a future boot-protocol swap, §6) changes *one*
    strategy, not four call sites.

### Non-Goals
- **No boot-protocol change.** x86_64 keeps `bootloader`-crate BiosBoot;
  x86_32 keeps Multiboot v1 + QEMU `-kernel`. Full *mechanism* parity is
  deliberately deferred (§6): the `bootloader` crate is x86_64-only (§2.1),
  QEMU `-kernel` speaks Multiboot v1 only, and GRUB v1 cannot load ELF64 —
  unifying mechanisms means writing our own bootloader or moving both to
  Multiboot2. Those are features, not refactors.
- **No kernel code changes.** Neither `arch/x86_64/src/main.rs` nor
  `arch/x86_32/src/main.rs` (nor `boot.S`) changes. Nothing in `kernel/` or
  `system/` changes.
- **No CI job restructuring.** The only workflow change contemplated is the
  optional ISO artifact in M4; artifact paths today keep working (§2.9).
- **No change to the GRUB toolchain requirement** (`grub-mkrescue`,
  `xorriso`, mtools) for the x86_32 flashable image. AMENDED during
  execution: the plan first claimed the script's `mcopy` check was dead
  weight — wrong. The script never calls `mcopy` directly, but
  `grub-mkrescue` invokes `mformat` (mtools) internally; CI proved it
  (§2.7 amendment). The ported checks probe `mformat` directly instead.

## 2. Verified baseline facts (2026-11-01, paths)

1. **`bootloader` 0.11 is x86_64-only.** Verified in the vendored registry
   source of `bootloader-0.11.17`: the doc line reads "An experimental
   x86_64 bootloader"; `KERNEL_FILE_NAME` is hardcoded `"kernel-x86_64"`;
   `disk_image.rs` invokes objcopy with `--binary-architecture=i386:x86-64`;
   every helper crate is named `bootloader-x86_64-bios-*`. The BiosBoot
   mechanism *cannot* boot the i686 kernel as-is. (Side fact: image creation
   shells out to objcopy — a hidden tool dependency, pre-existing.)
2. **The runner today** is `arch/x86_64-runner/` — a standalone workspace
   (`[workspace]` marker + own `Cargo.lock`, excluded from the root
   workspace members), package name `runner`, 30-line `main.rs`:
   `bootloader::BiosBoot::new(kernel).create_disk_image(<stem>-<arch>.img)`
   next to the kernel ELF, then spawns
   `qemu-system-x86_64 -drive format=raw,file=… -debugcon stdio -no-reboot
   -no-shutdown -d cpu_reset`. Referenced from: `xtask/src/main.rs:66`
   (`--manifest-path` spawn), `arch/x86_64/run.sh:5`, and four AGENTS.md
   lines (44, 62, 64, 205).
3. **The x86_32 run path duplicates itself.** `arch/x86_32/run.sh` and
   `xtask/src/main.rs::launch_multiboot` both spawn
   `qemu-system-i386 -kernel <elf> -debugcon stdio -display none
   -no-reboot -no-shutdown`. The x86_32 "image" for run purposes *is* the
   kernel ELF (QEMU's built-in Multiboot v1 loader).
4. **A third and fourth copy live in the test harness.**
   `tests-integration/src/disk_image.rs` calls `bootloader::BiosBoot`
   directly (own `bootloader` dep in `tests-integration/Cargo.toml`), and
   `tests-integration/src/qemu.rs:49-92` builds its own command, branching
   on `Arch::boots_via_disk_image()` (`arch.rs:66`). Test flags:
   `-debugcon stdio -display none -monitor unix:… -no-reboot -serial none
   -parallel none` — note **no `-no-shutdown`** (a triple-fault must exit
   QEMU rather than park it; see `plans/game-triple-fault-fix.md`).
5. **Flag-set deltas across the four call sites today:**

   | Call site | boot args | display | `-no-shutdown` | extras |
   |---|---|---|---|---|
   | x86_64 runner (interactive) | `-drive` | **window** | yes | `-d cpu_reset` |
   | xtask `launch_multiboot` (interactive) | `-kernel` | none | yes | — |
   | `arch/x86_32/run.sh` | `-kernel` | none | yes | — |
   | tests, both arches | per-arch | none | **no** | `-monitor unix`, `-serial none`, `-parallel none` |

   All four share `-debugcon stdio -no-reboot`.
6. **Interactive x86_32 is currently unusable by keyboard.** The shell reads
   PS/2 scancodes; with `-display none` there is no window to type into
   (stdio is debugcon-output-only, VGA text at 0xB8000 is invisible).
   Unifying "interactive" on the x86_64 convention (window shown) is an
   intentional bug fix, recorded here so it is not mistaken for drift.
7. **The GRUB path** is `arch/x86_32/build-image.sh` (bash), invoked only by
   `xtask/src/main.rs::build_grub_image`. It checks for
   `grub-mkrescue`/`grub2-mkrescue`, `xorriso`, **and `mcopy`**, but never
   calls `mcopy` directly — the check is nonetheless load-bearing:
   `grub-mkrescue` itself invokes `mformat` (mtools), proven when the new
   CI job without mtools failed with "grub-mkrescue: error: \`mformat\`
   invocation failed". Output:
   `target/rosx-i686/<profile>/rosx-x86_32.img` (an ISO9660 image; `dd`-able
   to USB per its own message).
8. **`xtask image` is asymmetric:** for x86_32 it builds the GRUB ISO; for
   x86_64 it prints "produced by `cargo xtask build`" (`main.rs:220-233`).
   `ImageKind::BiosBoot | Multiboot` (`xtask/src/arch.rs:6-9`) is the
   in-tree name of the divergence; `ArchSpec.qemu_binary` is a second copy
   of boot knowledge, next to `tests-integration`'s `Arch::qemu_binary`.
9. **CI today:** `ci.yml` uploads `target/rosx/release/rosx-x86_64.img` for
   x86_64 and the kernel ELF + `rom.img` for x86_32; no ISO is built in CI.
   `arch/x86_64/DEBUG_SETUP.md:147` expects the debug-profile image at
   `target/rosx/debug/rosx-x86_64.img` — a path derived from
   `<stem>-<arch>.img` that must not change.
10. **Nested same-workspace cargo is safe.** `cargo run -p rosx` executes
    `run.sh`, which spawns a *second* cargo for the runner. AMENDED at M1:
    the original reasoning ("keep the runner a standalone workspace to
    dodge a target-dir lock hazard") was empirically wrong twice over —
    (a) `cargo xtask build` is itself a nested same-workspace cargo chain
    (outer `cargo run -p xtask` spawns `cargo build …` against the shared
    target dir, sequentially, and works); and (b) cargo *refuses* a
    path-dependency from a member onto a package that keeps its own
    `[workspace]` marker inside the workspace directory ("multiple
    workspace roots found in the same workspace", hit on the first M1
    gate). §3.2 re-decided: the runner becomes a root-workspace member.
11. **Repo rules this plan honors** (AGENTS.md): no comments (assembly
    exception not applicable — no asm moves); no test-only production
    surface (every runner API gets a real caller in xtask, run.sh, or the
    harness; the runner's own `#[cfg(test)]` tests cover pure logic only);
    commit messages carry no prefix; `kernel/` untouched, so the
    dependency-rule paragraph is unaffected.

## 3. Design decisions

### 3.1 Naming and placement
Rename `arch/x86_64-runner/` → **`arch/x86-runner/`**, package
`runner` → **`x86-runner`** (lib `x86_runner`). The name follows the
`x86-generic` precedent: named for the family it serves, not one port.
The old name would be a lie the moment the x86_32 strategy lands.

### 3.2 Topology (amended at M1): root-workspace member with a lib + bin
`arch/x86-runner/` is a **root workspace member** (its `[workspace]`
marker and own `Cargo.lock` removed — cargo forbids member path-deps onto
nested workspace roots, §2.10). It is consumed two ways:
- **As a path-dependency lib** by `xtask` and `tests-integration`
  (`x86-runner = { path = "../arch/x86-runner" }`) — image creation and
  QEMU-command building happen *in-process*, removing both nested cargo
  spawns from xtask (`build_image`, and `launch_multiboot` becomes a
  `Command` it can exec) and the direct `bootloader` dep from the harness.
- **As a binary** from `run.sh` (both ports) via
  `cargo run --manifest-path …/arch/x86-runner/Cargo.toml -- <kernel> <arch>`;
  nested sequential cargo on the shared target dir is what xtask already
  does daily (§2.10). The bin `main.rs` stays a thin argv→lib shim.

Rejected: deleting the bin and rewriting `run.sh` to call xtask — xtask's
`run` subcommand *rebuilds apps+ROM first*, wrong job for
`cargo run -p rosx`.

### 3.3 The API (the whole crate surface)
```rust
pub enum Arch { X86_64, X86_32 }              // name(), from_name(), qemu_binary()
pub enum BootMedium {                          // what QEMU is pointed at
    DiskImage(PathBuf),                        //   x86_64: BiosBoot image
    MultibootKernel(PathBuf),                  //   x86_32: the kernel ELF itself
}
pub enum LaunchMode { Interactive, Headless }

pub fn run_medium(kernel_elf: &Path, arch: Arch) -> BootMedium;      // pure classification
pub fn prepare_run_medium(kernel_elf: &Path, arch: Arch) -> Result<BootMedium, ImageError>;
pub fn create_bios_disk_image(kernel_elf: &Path) -> Result<PathBuf, ImageError>;
pub fn create_grub_image(kernel_elf: &Path, out: &Path) -> Result<(), GrubTooling>;
pub fn qemu_command(arch: Arch, medium: &BootMedium, mode: LaunchMode) -> Command;
```
- `qemu_command` returns a *builder*: `-debugcon stdio` + mode flags +
  the boot-medium arg; callers add their own knobs (tests: monitor socket,
  synthetic IDE drives, stdio piped; nothing in the runner knows these).
- **Canonical flag sets** (single source of truth replacing the §2.5 table):
  - `Interactive`: `-debugcon stdio -no-reboot -no-shutdown`, window
    shown on both arches (§2.6 fix), `-d cpu_reset` kept for x86_64 only
    (preserves today's diagnostics, adds none to x86_32).
  - `Headless`: `-debugcon stdio -display none -no-reboot` and
    **deliberately no `-no-shutdown`** — verbatim preservation of the test
    suite's current semantics (§2.4).
- `create_grub_image` ports `build-image.sh`'s real work to Rust: temp
  tree, two-file `grub.cfg`, spawn `grub-mkrescue`/`grub2-mkrescue`, with
  the same friendly missing-tools message; the mtools check survives as a
  direct `mformat` probe (§2.7 amendment). This runs only from
  `xtask image x86_32` — CI paths that don't have GRUB never touch it.

### 3.4 Who owns which "Arch"
Three tables exist today (xtask `ArchSpec`, tests `Arch`, runner argv
strings). After the refactor:
- **`x86_runner::Arch` owns boot knowledge**: qemu binary, boot medium,
  flags, image creation.
- `xtask::ArchSpec` keeps build-path knowledge (package, target specs, dir
  names) and drops `qemu_binary`/`image_kind` in favor of one
  `boot: x86_runner::Arch` field.
- `tests-integration::Arch` keeps test-path knowledge and delegates
  `qemu_binary()` and `boots_via_disk_image()` to the runner (the latter
  becomes `matches!(run_medium(…, self.boot()), BootMedium::DiskImage(_))`,
  so the existing unit test `arch.rs:108` keeps its meaning).
Full merger of the three tables is out of scope (§6) — the *build* table
and the *harness* table legitimately differ; only boot knowledge converges.

### 3.5 Artifact names stay stable
`rosx-x86_64.img` naming (`<stem>-<arch>.img` next to the kernel ELF) is
kept — CI uploads and `DEBUG_SETUP.md` (§2.9) depend on it. x86_32 ISO
keeps `rosx-x86_32.img` as produced today (§2.7). `cargo xtask image`
becomes symmetric in *semantics*: on x86_64 it (re)creates the BiosBoot
disk image (today a message, §2.8); on x86_32 the GRUB ISO.

## 4. What moves, what stays

| Piece | Verdict | Evidence |
|---|---|---|
| `arch/x86_64-runner/` | **moves + renames** to `arch/x86-runner/`, gains lib API | §2.2 |
| `xtask::build_image` | **in-process lib call** (BiosBoot via runner lib) | §3.2 |
| `xtask::launch_multiboot` | **deleted**; launch via `run_medium` + `qemu_command` | §2.3 |
| `xtask::build_grub_image` + `build-image.sh` | bash logic **re-implemented in the runner lib** (M4); xtask calls `create_grub_image` | §2.7 |
| `arch/*/run.sh` (both) | **delegated**: one-line spawn of the renamed runner bin | §3.2 |
| `tests-integration/src/disk_image.rs` | **deleted**; runner lib called directly | §2.4 |
| `tests-integration/src/qemu.rs` | base command from `qemu_command(…, Headless)`; test knobs (monitor, IDE disks, stdio) stay local | §3.3 |
| `bootloader` dep in `tests-integration` | **goes away** (comes transitively via the runner) | §3.2 |
| `ArchSpec.qemu_binary`, `ArchSpec.image_kind`, `Arch::qemu_binary`, `Arch::boots_via_disk_image` | **delegate or fold** into `x86_runner::Arch` | §3.4 |
| Kernel boot code (both `main.rs`, `boot.S`), `ImageKind` at the *mechanism* level | stays | Non-Goals |
| `debug-qemu.sh` et al. | stay as-is (they spawn QEMU with GDB knobs on an unchanged image path) | §2.9 |
| CI workflows | unchanged (optional ISO step in M4 only) | §2.9 |

## 5. Milestones (each one commit; approve-and-go per step)

### M1 — Crate rename + library extraction, x86_64 path only (zero behavior change)
- `git mv arch/x86_64-runner arch/x86-runner`; package `x86-runner`; split
  `main.rs` into `lib.rs` (`create_bios_disk_image`, `qemu_command` for
  `X86_64/DiskImage/Interactive`, argv shim) + thin bin.
- `xtask`: add path dep; `build_image` calls the lib (both `--no-run` and
  launch paths). `arch/x86_64/run.sh`: repoint `--manifest-path`.
  AGENTS.md lines 44/62/64/205 updated in the same commit (docs must not
  lie between milestones).
- **Gate:** `cargo xtask build` produces `target/rosx/release/rosx-x86_64.img`
  byte-comparable to a pre-change build (**`unverified`** baseline captured
  by the orchestrator before M1); `cargo xtask test` (x86_64, full QEMU
  suite); `cargo run` boots to the shell (**`unverified`** — manual).
- **Commit:** `Rename the x86_64 runner to x86-runner and expose it as a library`

### M2 — x86_32 boot strategy in the runner; xtask + run.sh converge
- `lib.rs`: `Arch::X86_32`, `run_medium` → `MultibootKernel(kernel_elf)`,
  `qemu_command` headless+interactive for `-kernel`; unit tests for
  `Arch::from_name` and medium selection.
- `xtask`: `build`/`run` `ImageKind` match arms collapse into
  `run_medium`/`qemu_command`; `launch_multiboot` deleted; `ArchSpec`
  gains `boot: Arch`, loses `qemu_binary`/`image_kind`.
  `arch/x86_32/run.sh`: one-line delegate (§3.2).
- **Behavior change recorded:** interactive x86_32 now shows a QEMU window
  (keyboard usable, §2.6).
- **Gate:** `cargo xtask test` for both arches
  (`ROSX_ARCH=x86_32 cargo test -p tests-integration -- --test-threads=1`);
  `cargo xtask build x86_32`; manual `cargo xtask run x86_32` to the prompt
  (**`unverified`**).
- **Commit:** `Move x86_32 QEMU launching into the shared x86 runner`

### M3 — The test harness boots through the runner
- `tests-integration`: dep on `x86-runner`; `disk_image.rs` deleted;
  `qemu.rs` base from `qemu_command(…, Headless)` + its own monitor/IDE/
  stdio knobs; `Arch::qemu_binary`/`boots_via_disk_image` delegate (§3.4).
- **Gate:** full `cargo xtask test` + `ROSX_ARCH=x86_32` integration suite.
  This is the refactor's true oracle: any flag drift fails loudly.
- **Commit:** `Boot the integration suite through the shared x86 runner`

### M4 — Symmetric `cargo xtask image`, GRUB logic in Rust
- `lib.rs`: `create_grub_image` (§3.3), host-testable pieces (grub.cfg
  bytes, tool-resolution fallback order) unit-tested on host.
- `xtask image`: x86_64 arm re-creates the disk image; x86_32 arm calls
  `create_grub_image`; `build-image.sh` deleted; usage line updated.
- CI step approved 2026-11-01 and added: `build-artifacts.yml` runs
  `cargo xtask image x86_32` after installing the GRUB toolchain and
  uploads `rosx-x86_32-iso`.
- **Gate:** `cargo xtask image x86_32` produces a bootable ISO —
  `qemu-system-i386 -cdrom <iso> -debugcon stdio` reaches the prompt
  (**`unverified`** — needs grub-mkrescue locally); missing-tools message
  verified by shadowing PATH; `cargo xtask image` (x86_64) writes the `.img`.
- **Commit:** `Standardize cargo xtask image on both arches with a Rust GRUB builder`

### M5 — Documentation sweep
- AGENTS.md: build/run block — `cargo xtask image [arch]` symmetric line;
  the "No bootimage" paragraph names `arch/x86-runner` and documents both
  strategies; `arch/` tree line 44 updated; the x86_32 section's GRUB note
  stops referencing bash. Verify with `grep -rn "x86_64-runner\|build-image.sh"`.
- **Commit:** folds into whichever commit last changes the fact, per repo habit.

## 6. Deferred, recorded not dropped
- **Native RosX bootloader (Option B).** One loader for both widths is the
  only path to *mechanism* parity and stays open because the runner API is
  mechanism-agnostic: swapping a `BootMedium` variant is the whole xtask/
  tests/CI impact.
- **Multiboot2 + shared MBI parser in `arch/x86-generic` (Option C2).**
  Same swap story. Noted constraints: QEMU `-kernel` is v1-only; GRUB MBI2
  framebuffer reliability varies.
- **Single merged Arch table.** Boot knowledge converges here; build/harness
  tables merging is speculative until a third arch appears.
- **ISO boot integration test** (M4 gate automated as a `tests-integration`
  case) — candidate once `create_grub_image` has proven out.
- **`-d cpu_reset`** stays x86_64-only; promoting it to canonical (or
  deleting) is a diagnostics decision for whoever next debugs a reset.

## 7. Risks

| # | Risk | Mitigation |
|---|------|------------|
| 1 | Flag drift changes QEMU behavior under the tests | §3.3 canonical sets are transcribed from the §2.5 table, per-mode, deliberately unequal (headless keeps no `-no-shutdown`); M3's gate is the full dual-arch QEMU suite |
| 2 | Nested-cargo lock deadlock via `run.sh` | AMENDED at M1: nested sequential same-workspace cargo is the xtask pattern and holds no lock while its child runs; M1/M2 gates still run `cargo run` to prove it end-to-end |
| 3 | GRUB re-implementation subtly differs from `build-image.sh` | Materialized: the port dropped the mtools probe believing the script's check dead; CI failed on grub-mkrescue's internal `mformat` (§2.7 amendment). Restored with a probe of the actually-invoked tool; the real toolchain (CI's) is the standing oracle |
| 4 | Artifact path breaks CI or debug docs | §3.5 freezes both paths; `if-no-files-found: error` in CI catches any slip immediately |
| 5 | Interactive x86_32 window surprises stdio-trained users | It fixes an unusable keyboard path (§2.6); PR body records it; Headless mode remains for scripts |
| 6 | `x86-runner` path-dep on an external workspace misbehaves | Already the shape of the `bootloader` ecosystem (bin-component crates located at runtime); M1 gate compiles xtask + tests against the lib immediately |

## 8. Verification matrix

| Command (cwd) | Proves | Result (2026-11-01) |
|---|---|---|
| `cargo xtask build` / `cargo xtask build x86_32` (root) | both full build chains incl. in-process image creation | **green** at every milestone |
| `cargo xtask test` (root) | x86_64 QEMU suite through the runner (M3+) | **green** (M1, M3) |
| `ROSX_ARCH=x86_32 cargo test -p tests-integration -- --test-threads=1` | x86_32 QEMU suite through the runner | **green** (M2, M3) |
| `cargo run` / `cargo xtask run [arch]` | interactive boot to shell, window behavior | chain proven to QEMU display init on both arches (headless box: `gtk initialization failed` is the terminal proof); full window boot **unverified** here |
| `cargo xtask image` / `cargo xtask image x86_32` (+ `-cdrom` boot) | symmetric artifacts, bootable ISO | x86_64 **green**; x86_32 missing-tools UX **green** (no xorriso on this box); full-toolchain build verified via the `build-artifacts.yml` re-run after the §2.7 amendment; `-cdrom` boot of the artifact remains manual |
| image byte-compare pre/post M1 | BiosBoot call unchanged | **identical** at M1, M2, M4 vs pre-change baseline |

## 9. Interfaces summary
After M4 the runner crate's public surface is §3.3's seven items
(`Arch`, `BootMedium`, `LaunchMode`, `run_medium`, `prepare_run_medium`,
`create_bios_disk_image`, `create_grub_image`, `qemu_command` plus the two
error types); its binary keeps today's argv contract
`<kernel-binary> <arch> [--no-run]`.
xtask's user-facing contract changes only in the honest direction:
`cargo xtask image` now means "produce the flashable artifact for this
arch" on **both** arches. No kernel, treaty, protocol, or app interface
changes anywhere in this plan.
