# RosX - Agent Context Document

> **Purpose:** compact context for AI coding agents. Long design prose lives in `docs/architecture/`; this file keeps orientation, commands, invariants and the working rules.

## Purpose and platform status

**RosX** is a learning/hobby OS in Rust (edition 2024, nightly, minimal assembly), aiming at diverse hardware and a multi-purpose OS that can eventually run network applications. Scope today: scheduling, interrupts, IPC, ELF user space, block/VFS/filesystem stack, an Ethernet/ARP/IP stack.

- **x86_64** — bootstrap platform, complete.
- **x86_32** — complete (custom BIOS chain on the unified boot disk, `arch/x86_32/`); 486-class hardware is a maintained baseline.
- **ARM, RISC-V, m68k** — future; no `arch/` directories yet.

x86_64 is the starting point due to tooling, not the focus: keep `kernel/` portable. The workspace layout is `Cargo.toml` plus the directory names (`kernel`, `system`, `usrlib`, `protocol/*`, `fs/*`, `arch/*`, `apps/*`).

## Commands

```bash
cargo xtask check                    # THE GATE: check-style, check-arch, check-isa, build surfaces, unit tests (QEMU integration excluded — use cargo xtask test)
cargo xtask verify                   # MERGE EVIDENCE: the gate + full integration on BOTH arches + boot-test (~40 min)
cargo xtask build [x86_64|x86_32]    # apps + ROM + kernel + boot artifacts (FAT boot disk needs mtools)
cargo xtask run [x86_64|x86_32]      # boot QEMU to the shell prompt
cargo xtask test [--skip-integration]
cargo xtask decisions                # open decisions and every todo, by priority
cargo xtask install-hooks            # git pre-commit hook running the gate (the Claude Stop hook in .claude/settings.json runs it too)
```

- `cargo xtask help` lists every subcommand; CI recipes are in `.github/workflows/ci.yml` and `build-artifacts.yml`.
- Integration tests: `cargo xtask test` builds the release kernel and sets `ROSX_KERNEL_ELF`; by hand, `ROSX_KERNEL_ELF=<prebuilt release kernel> cargo test -p tests-integration -- --test-threads=1` (add `ROSX_ARCH=x86_32` for the 32-bit port). With neither variable set the harness fails loudly; `ROSX_QEMU_PROFILE=dev|release` is the explicit opt-in to a harness-built kernel.
- Unit tests: `cargo test -p kernel -- --test-threads=1`, `cargo test -p rxfs`, `cargo test -p usrlib`.
- `cargo build --workspace` is intentionally unsupported: bare-metal bins and PIE apps cannot compile for the host.
- Boot media, QEMU wiring and harness facts: `docs/architecture/boot-media-and-harness.md`.

## Invariants (index)

Each line links the file holding the detail; every file starts with its own invariants.

- `kernel/` is platform-agnostic and `no_std`, depends only on `system`; all hardware goes through HAL traits (`Cpu`, `ElfArch`, `PortIo`). — `docs/architecture/scheduling-and-interrupts.md`
- Kernel code runs with IF=0; level-triggered IRQ lines stay masked until the driver calls `IrqClear`; `Kernel::maybe_resched()` is the only forced context-switch site; `[IRQGUARD]` must stay silent. — `docs/architecture/scheduling-and-interrupts.md`
- Futures and IPC are notification-driven; the kernel never interprets IPC payloads. — `docs/architecture/ipc-and-futures.md`
- A task's exit status rides its completion future; future slots are owned by loaded-image groups and swept with the group, and a dying task's own futures and connections are reaped on both `Exit` and `Kill`. — `docs/architecture/exit-status-and-ownership.md`
- User memory is owned by loaded-image groups; task stacks are not freed by group reclaim. — `docs/architecture/memory.md`
- The ELF image and argv are group-owned, loaded eagerly at the load call; the kernel retains no ELF bytes after a load. — `docs/architecture/memory.md`
- Boot is plan-driven by `system::boot_plan`; failure policy is log-and-continue. — `docs/architecture/boot-plan.md`
- Block devices come only from `blockd`; the ATA face is a diagnostic sharp knife; ports are won through `PortAcquire`; the probe's DrivePresent rule (`!BSY && !DRDY` = nothing attached) is judged only while waiting for DRQ — during reset/select a present drive reads the same and must not be refused. — `docs/architecture/disk-and-blockd.md`
- `vfsd` owns no storage and speaks the `Engine` treaty; the mount table is the root. — `docs/architecture/vfs.md`; engines — `docs/architecture/fs-engines.md`
- Ethernet lives in `ne2kd`, L3 in `ipd`; pushed data handles stay parked until read. — `docs/architecture/network-and-ip.md`
- Input policy lives in `inputd`, device semantics in driver daemons. — `docs/architecture/input.md`
- `kstatsd` is kernel-hosted; the ROM is never a VFS volume. — `docs/architecture/kstatsd-and-rom.md`
- Shell semantics. — `docs/architecture/shell.md`
- `ted` is a disk app that saves by delete-then-recreate. — `docs/architecture/text-editor.md`
- x86_32 stays on the 486 baseline: pinned feature floor, no FP, no `cpuid`, atomics of at most 32 bits. — `docs/architecture/486-baseline.md`; hardware checklist — `docs/architecture/real-hardware-x86_32.md`

## Architecture principles (who owns what)

- **Domain knowledge is owned by the allocating subsystem.** A kernel resource's identity, layout and validity rules live with the subsystem that allocates and validates it; OS policy never lives in infrastructure crates.
- **The kernel is the single authority on invariants.** Validation happens once, kernel-side; user space carries opaque tokens and neither re-checks nor duplicates kernel invariants.
- **The shared treaty holds the contract, not the mechanism.** `system` holds numbers and layouts only.
- **Translation lives at the boundaries.** The kernel edge encodes domain to wire; the user edge decodes wire to user view.
- **User-visible APIs expose no kernel internals.** Debug aids are kernel-side and never part of the ABI.

## Code style (judgement-based)

- Prefer safe Rust; `unsafe` only when unavoidable (hardware access, inline assembly), in the smallest possible blocks behind safe wrappers.
- Hardware abstraction: platform code goes in `arch/<platform>/`; x86-family code that is not bit-width-specific goes in `arch/x86-generic/`; use traits (`Cpu` is the reference).
- No test-only production surface: every production item has a non-test caller; test scaffolding lives in the `#[cfg(test)]` module of the unit under test; production API with no caller is deleted.
- Subsystems are pluggable by trait at boot time (scheduling already is: `SchedulingAlgorithm` via `KConfig`); memory is not pluggable yet (low-priority goal).
- Write unit tests for every kernel module (`kernel/src/scheduler/mlfq_strategy.rs` is the model); verify behaviour in QEMU where a unit test cannot.
- Follow standard Rust conventions, small focused functions, type safety over raw integers and pointers, descriptive names.
- `kernel/` allows no third-party crates, only the workspace crate `system`.

## Rules enforced by `cargo xtask check`

- **Comments:** none, except a short (at most 6 lines) `///`/`//!` on an item containing `unsafe`/`asm!`/`global_asm!` (or an `unsafe fn`/`unsafe impl`) that links its `docs/rationale/<slug>.md`; assembly (`*.S`, `asm!`/`global_asm!` bodies) keeps its mandatory register and calling-convention comments. Today's comments are frozen in `xtask/style-baseline.txt`, which only ratchets down.
- **`#[allow]`** is banned; use `#[expect(lint, reason = "...")]` and never without the maintainer's approval for that specific case. The existing `#[allow]` attributes are baselined.
- **`unsafe`** counts per crate are baselined; raising a count needs a record in `docs/decisions/`.
- **Dependency arrows** and the 486 feature floor: `xtask/arch-rules.txt`.
- **32-bit hazards** (`f32`, `f64`, `AtomicU64`, `AtomicI64` in 32-bit-reachable crates), **handle kind guards** (`usrlib/tests/handle_kind_guards.rs`), **warnings as errors** (`[workspace.lints.rust]`), **486 ISA audit** (`check-isa`).
- **check-style surveys `git ls-files` only**: a new source file counts toward the unsafe ratchet and its rationale links only after `git add`.
- `RUSTFLAGS` in the environment replaces the user-target rustflags that carry `linker.ld`; never export it when building user ELFs.

## Autonomy

- Proceed without asking whenever a choice is covered by a written invariant, a gate, or an existing pattern in the repo.
- Commit per small, reviewable step when `cargo xtask check` is green, with no message prefix, and push to the working branch. Never create a PR unless asked.
- End every plan step with the `retro` skill's step-end mode, reported with the step's commit; a lesson changing rules or docs is a proposal that lands only after approval, in its own commit.
- Stop and ask only for: breaking an invariant or the architecture principles; new third-party dependencies; ABI or wire changes in `system` or `protocol/*`; widening the 486 feature floor; claims that need hardware evidence; deleting user data or history; anything outward-facing beyond the working branch.
- Gate failures are fixed at the cause; never skip, loosen or quarantine a gate to get green.
- PR descriptions follow `.github/pull_request_template.md`; creating the PR stays a manual, user-requested act.

## Decision log

- A choice not needed now becomes a file in `docs/decisions/` (descriptive kebab-case name, `status: open`, a `revisit-when` trigger; see its README); work continues on the chosen default.
- Work noticed but out of scope becomes a file in `docs/todo/`; delete it in the commit that completes the work.
- Raising a gate baseline needs a decision record in the same commit (`check-style --update-baseline --allow-increase "<reason>"` writes it).
- Design rationale for one piece of low-level code goes in `docs/rationale/`.

## Toolchain notes

- `rust-toolchain.toml` pins **nightly** with `rust-src`; custom targets live in `arch/x86_64/` (`rosx.json`, `rosx-user.json`) and `arch/x86_32/` (`rosx-i486.json`, `rosx-i486-user.json`).
- No bootimage and no carrier: both arches chain-boot the unified disk that `arch/x86-runner` packs from `arch/x86-bootloader`'s stage1/stage2 (per-arch stage2 flavor), attached at IDE index 0 with `bootindex=1`.
- `gh pr edit` on `rghisi/rosx` fails with a Projects (classic) GraphQL error; update PRs through the REST API: `gh api -X PATCH repos/rghisi/rosx/pulls/<n> -f title="..." -F body=@<file>`.
- Run `cargo xtask test` and `cargo xtask verify` one at a time: the QEMU suites share `target/` and contend with any suite already running.
- Always run long suites with their output captured to a file and the exit code recorded (`cmd > out.log 2>&1; echo $?`): never derive a verdict from piped or truncated output. A truncated tail looks like a pass — a 40-minute `verify` that died mid-run once made `rosx-x86_32` look like it passed. If a suite's output was truncated, the suite never ran.
- `cargo xtask verify` needs a quiet host: its integration boots are load-sensitive (the flake rows catalogued in `plans/integration-flakiness-assessment-2026-11-20.md` fail only under contention). Before trusting a verdict, confirm no other suite is live (`pgrep -f 'cargo test|xtask test|qemu-system'`, load average against `nproc`); when the box is busy, chain verify behind a quiet-machine wait instead of rerunning into contention.
