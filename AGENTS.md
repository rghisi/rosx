# RosX - Agent Context Document

> **Last Updated:** 2026-11-15
> **Purpose:** Compact, always-accurate context for AI coding agents working on RosX.

---

## Project Overview

**RosX** is a learning/hobby OS project written in Rust, aiming to be a multi-platform operating system capable of running on diverse hardware - from microcontrollers to m68k computers to x86_64 systems. The goal is to create a multi-purpose OS that can run network applications such as web servers.

### Key Characteristics
- **Language:** Rust (with minimal assembly for architecture-specific code)
- **Target Platforms (status):**
  - **x86_64** — bootstrap platform, complete.
  - **x86_32** — complete (Multiboot v1/GRUB, `arch/x86_32/`); 486-class hardware is a maintained baseline.
  - **ARM, RISC-V, m68k** — future (no `arch/` dirs exist yet; m68k is feasibility-stage only).
- **Scope:** Full-featured OS — scheduling, interrupts, IPC, ELF user-space; networking is a goal, not yet implemented.
- **Project Type:** Learning/hobby project with practical goals

**Platform Strategy:** x86_64 is the starting point due to tooling, not the focus — keep `kernel/` portable.

---

## Project Structure

```
rosx/                        # Cargo workspace (edition 2024, nightly)
├── kernel/                  # Platform-agnostic core (see submodules below)
│    scheduler/ (SchedulingAlgorithm: MLFQ + FIFO)
│    memory/   (global allocator; FreeList + BitmapChunk)
│    collections/ (arena)  handle/ (5 newtypes: task/future/ipc-connection/ipc-binding/ipc-data)
│    elf/ ipc/ (mailbox, data) future/ syscall/ kstatsd task*  platform/ (cpu, elf, interrupt, output, portio HAL traits + per-task port grants, acquired exclusively from the arch pool)  kconfig  rom/
├── system/      # ABI treaty: syscall numbers, wire layout, ROM image layout, boot plan grammar, error numbers
├── usrlib/      # User-space libc: allocator (binned slabs, global default), syscall, exit, out, arch/{x86_64,x86_32}
├── protocol/                # Service RPC treaties, one sub-crate each (workspace glob protocol/*)
│    vfs/         # package vfs-protocol: wire codec + typed client
│    input/       # package input-protocol: wire codec
│    ata/         # package ata-protocol: wire codec + typed client
│    block/       # package block-protocol: generic block-device family (reserved opcode band) + manager contract (blockd namespace) — wire codecs + dial-by-name clients
│    ip/          # package ip-protocol: IP treaty (Status/Config/Neigh/Subscribe/SendIp + datagram/unreachable pushes) + typed client
├── fs/
│    core/        # engine treaty (trait Engine + device treaty BlockDevice + FsError/Kind/DirItem/MetadataView), no deps, shared by rxfs, rxfat, ramfs and vfsd
│    rxfs/        # no_std filesystem library (vfsd's engine): superblock/bitmaps/inodes/extents/dirs, leak-safe sync, check()
│    fat/         # package rxfat: no_std FAT16/FAT32 read/write engine (8.3 names, total boot-sector parser), vfsd's second mountable engine
│    partition/   # package rxpart: total MBR parser, curation rule and encoder, blockd's exposure oracle
│    ramfs/       # no_std in-memory filesystem engine, rxfs-shaped (lookup/open/create/mkdir/rmdir/read/write/close/remove/readdir/stat) over a path-keyed store
├── arch/
│    x86_64/        # entry, cpu, interrupts, framebuffer, terminal_fonts, *.S (global_asm)
│    x86_32/        # entry, cpu, interrupts, boot.S (Multiboot v1)
│    x86-generic/   # shared x86-family mechanism: port executor, VGA text buffer, debug console, ANSI parser, Multiboot v1 memory decode
│    x86-runner/    # workspace member: kernel → boot medium + QEMU launcher for both ports (NOT bootimage)
│    x86-86box/     # data-and-scripts (no crate): era-VM template, GRUB Legacy media recipes, provenance manifest, runbook
└── apps/
     [workspace members]  shell, init, dummy, pi, randomd, echo, bulk, pong, ping, mem, ps, vfsd, vfs, snake, tetris, conway, threads, waitset, stress, inputd, ps2d, atad, ata, blockd, bench, fsbench, format, mount, attach, rm, cp, mkdir, rmdir, exiter, pcid, pci, ne2kd, ne2k, arp, ipd, ip, leaky
```

---

## Build & Run

Canonical recipes: `.github/workflows/ci.yml` (build + test) and `build-artifacts.yml` (disk images).

### x86_64
```bash
cargo xtask build              # ELF apps + ROM image + kernel + BIOS boot image + FAT boot disk
cargo xtask apps               # ELF apps only
cargo xtask rom                # ROM image from built apps
cargo xtask test               # all host unit tests
cargo build -p rosx            # kernel only (from arch/x86_64 directory or with target)
cargo run -p rosx              # run.sh -> arch/x86-runner -> BiosBoot image + FAT disk -> QEMU
```
- `cargo run` uses the **custom runner** (`.cargo/config.toml` → `./run.sh`), which invokes `arch/x86-runner` to make a BIOS disk image with the `bootloader` crate's `BiosBoot` (attached on the **secondary** IDE bus with `bootindex`, so the kernel is still loaded via BIOS but the carrier never enters the guest namespace), then starts `qemu-system-x86_64`. **Not bootimage.** The `bootloader` crate's stages honor the BIOS-supplied drive number end to end, so the carrier works from any IDE slot.
- **FAT boot disk** — `cargo xtask build` (x86_64) also writes `target/rosx-boot-disk.img` (host `mformat`/`mmd`/`mcopy` from mtools), the same recipe as x86_32: a 64 MiB image whose MBR (stamped with `rxpart::mbr::encode_mbr`) carries one bootable FAT16 slice (type 0x06, LBA 63) with every user ELF at the root plus `/boot/kernel.elf` and `/boot/rom.img`. Both `cargo run` (when the image is present) and `cargo xtask run` attach it at `if=ide,index=0`, so the curated namespace is `ata0d0` with slice `ata0d0p0`; the on-disk `kernel.elf`/`rom.img` are forward-looking artifacts for a real GRUB boot, not mechanisms (the kernel uses its embedded ROM and its BIOS carrier).
- Both `cargo run` and `cargo xtask run` attach a persistent host-side 50 MiB IDE disk at `if=ide,index=1` (`$HOME/tmp/rosx-hdd.img`, created if missing, never truncated, survives VM restarts): `attach ATA` → `format ata0d1` → `mkdir /<dir>` → `mount ata0d1 /<dir>` → `ls /<dir>` (the mount point is a real root-volume directory that must exist before mounting; `attach ATA` registers the ATA controller with `blockd` — the single block-device provider whose namespace lives in memory, so every boot re-attaches; the persistent disk `ata0d1` has no partition table, so it stays a whole drive). Because the kernel carrier rides the secondary bus (outside atad's primary-channel port pool), the boot disk is the FAT volume `ata0d0`/`ata0d0p0` and the old `ata0d0p1`/`ata0d0p2` bootloader-slice footgun is gone. The headless integration harness keeps its own tempdir disks (`QemuSession::spawn_with_hdd_disk` for the persisting flow; `spawn_with_partitioned_hdd_disk` / `spawn_with_corrupt_partitioned_hdd_disk` hand-carve an MBR into the image first).
- Target spec: `arch/x86_64/rosx.json` — bare-metal `no_std`, `build-std` = core/alloc/compiler_builtins.
- User-space ELF apps (pi, randomd, snake, tetris, conway) are workspace members; build with `cargo xtask apps` or `cargo xtask build`.
- Boot is plan-driven: each arch port embeds only the ROM image, the `init` ELF, and `kstatsd`; `init` walks the arch's `BootPlan` (`system::boot_plan`, passed as init's argv) — services launch from ROM and wait their bind-name probe, `run` steps wait a tool's exit, `exec` hands the console over and init exits. Both ports boot the same 14-step plan (`rom:atad`, `rom:blockd`, `run rom:attach ATA`, `rom:vfsd`, `rom:inputd`, `rom:ps2d`, `run rom:mount ata0d0p0 /`, `service /randomd|/echo|/pong`, `service /pcid|/ne2kd`, `exec /shell`, `exec rom:shell`); `bulk` is not in the plan (the `ipc_bulk` test launches it from the shell, and bulk exits when done).
- The shell is a user ELF executed by init from the mounted FAT root, falling back to the ROM copy when the disk is missing. It keeps five built-ins (`ls clear rose sleep random`) and resolves every other prompt name through VFS, so disk/ROM files execute by name with arguments (`rose> snake fast`). A trailing `&` token backgrounds a launched program: the shell loads the ELF without waiting on its completion future, announces `<name> started`, and reaps finished jobs at every prompt boundary — a non-consuming `IsFutureCompleted` scan, then a `WaitFuture` that consumes the future (returning its registry slot, the one leak guard a detached launch needs) and prints `finished` / `exited with code N` / `panicked`, so notices surface at the next prompt, never asynchronously. Built-ins execute inside the shell task and cannot detach; `&` on one is announced and run in the foreground. Background output interleaves with the prompt: there is no tty or job control.
- `cargo build --workspace` is intentionally unsupported: bare-metal bins and PIE apps cannot compile for the host.

### x86_32
```bash
cargo xtask build x86_32       # ELF apps + ROM image + kernel (Multiboot v1, boot.S) + FAT boot disk
cargo xtask run x86_32         # boot QEMU (-kernel + the FAT boot disk at ide index=0) to the shell prompt
cargo xtask image x86_32       # GRUB bootable ISO (needs grub-mkrescue, xorriso, mtools)
cargo build -p rosx-x86_32     # kernel only (from arch/x86_32/ — its .cargo/config.toml supplies the rosx-i486 custom target)
```
- The kernel embeds the init ELF and the ROM image that `cargo xtask build x86_32` produces under `target/rosx-i486-user/release/`; `arch/x86_32/build.rs` asserts both are present before compiling.
- **FAT boot disk** — `cargo xtask build x86_32` also writes `target/rosx-i486-boot-disk.img` (host `mformat`/`mmd`/`mcopy` from mtools): a 64 MiB image whose MBR (stamped with `rxpart::mbr::encode_mbr`) carries one bootable FAT16 slice (type 0x06, LBA 63) with every user ELF at the root — all app names must fit the 8.3 stem, or the build says so — plus `/boot/kernel.elf` and `/boot/rom.img`. QEMU loads the kernel via `-kernel` and the kernel uses its embedded ROM, so the on-disk `kernel.elf`/`rom.img` are forward-looking artifacts for a real GRUB boot, not mechanisms. `cargo xtask run x86_32` attaches the image at `if=ide,index=0`, so the curated namespace is `ata0d0` with slice `ata0d0p0`; the persistent host disk stays at index=1 (`ata0d1`).
- The boot plan boots from that disk: the stack comes up from ROM, `run rom:mount ata0d0p0 /` root-mounts the FAT volume, `service /randomd|/echo|/pong …` start **from the mounted volume**, and `exec /shell` hands over to the shell **loaded from disk**; when the disk or its binaries are missing the service steps log `not available` and the plan's last step (`exec rom:shell`) lands on the ROM shell instead. The ROM carries only the bootstrap set (`BOOTSTRAP_APPS` in `xtask/src/arch.rs`: atad, blockd, attach, mount, vfsd, inputd, ps2d, shell — the `rom:` tokens plus the console stack) on both ports; x86_64 runs the identical plan on its own `target/rosx-boot-disk.img`.
- Integration tests run on x86_32 with `ROSX_ARCH=x86_32 cargo test -p tests-integration -- --test-threads=1`; the harness boots the kernel ELF directly, and every session on both arches attaches a **full-apps FAT master** (`tests_integration::fat_disk` — rxpart-stamped MBR + mtools, all ELFs found in the user release dir), so sessions disk-boot to the disk shell and resolve every CLI through the root mount; the ata suites keep their second-drive topology on top. `ipc_vfs` and `ata_env_spike` stay x86_64-only, `disk_boot` is x86_32-only (root mount, disk-shell hand-off, ROM fallback, and a test that boots the real `target/rosx-i486-boot-disk.img` when `cargo xtask build` produced it); `kstats_mem` runs unguarded on both arches (the wake-path race is fixed — `plans/x86_32-wake-race.md`); the `fsbench` integration test is `#[ignore]`d like the other benchmark tests — CI never runs benchmarks, and full-phase `fsbench` stays manual anyway until `plans/ipc-volume-wedge.md` is fixed, because sustained IPC volume kills the guest silently.
- Task stacks (`STACK_WORDS` in `kernel/src/task_stack.rs`) are sized from watermark measurements: 8192 words on x86_64, 12288 on x86_32 — the nested `vfsd`→`atad` format chain has a fixed depth of ~35 KiB (much of it the 4 KiB IPC receive chunks). The x86_32 chain measures nearly the same byte depth as 64-bit: the stack is dominated by fixed-size buffers, not register frames.

### Real hardware (x86_32)

See `docs/architecture/real-hardware-x86_32.md` for the boot recipe and the hardware verification checklist.

### Unit tests (run on host)
```bash
cargo xtask test               # all workspace unit tests
cargo test                     # default-members only
cargo test -p kernel -- --test-threads=1   # scheduler, memory, elf, ipc, future
cargo test -p rxfs             # filesystem library (add --features std for the FileDevice suites)
cargo test -p fs-core          # engine treaty crate (types + traits, no tests of its own)
cargo test -p ramfs            # in-memory filesystem engine (rxfs-shaped path-keyed store)
```

### Integration tests (run on host against QEMU)
```bash
cargo test -p tests-integration -- --test-threads=1                      # x86_64
ROSX_ARCH=x86_32 cargo test -p tests-integration -- --test-threads=1      # x86_32
```
- **Always set `ROSX_KERNEL_ELF` to a prebuilt release kernel** (CI does: `target/rosx/release/rosx`, `target/rosx-i486/release/rosx-x86_32`). Unset, the harness now fails with a message (`ROSX_QEMU_PROFILE=dev` is the explicit opt-in); `cargo xtask test` builds the release kernel and sets the variable itself (`tests-integration/src/kernel_build.rs`).

### xtask commands
```bash
cargo xtask build [arch] [--debug]  # ELF apps + ROM + kernel + boot artifact; x86_32 also writes the FAT boot disk (needs mtools)
cargo xtask apps [arch]             # Build ELF apps only
cargo xtask rom [arch]              # Pack the ROM image from the built user ELFs
cargo xtask run [arch] [--debug]    # Build and launch QEMU to the shell prompt
cargo xtask image [arch] [--debug]  # flashable artifact: x86_64 BiosBoot disk image, x86_32 GRUB ISO (needs grub-mkrescue, xorriso, mtools)
cargo xtask era-image x86_32        # era boot media for 86Box/hardware: GRUB Legacy 0.97-67 El Torito ISO + FAT16 HDD (needs mtools, sfdisk, xorriso, docker for the grub shell; see arch/x86-86box/README.md)
cargo xtask check-isa x86_32        # objdump audit: kernel + every user ELF must contain only 486-era mnemonics
cargo xtask check-isa --stages [dir] # audit the pinned GRUB Legacy stage binaries (default target/grub-legacy/stages; fetch via arch/x86-86box/scripts/fetch-grub-legacy.sh)
cargo xtask test [--skip-integration] # Run unit tests; --skip-integration excludes the QEMU tests-integration suite
```

## Development Guidelines

### When Working on RosX

1. **Check current state first:**
   - Read git status and recent commits

2. **Understand the architecture layer:**
   - `kernel/` should remain platform-agnostic
   - Architecture-specific code goes in `arch/[platform]/`
   - x86-family-specific but not bit-width-specific code lives in `arch/x86-generic/`; anything else stays in its port
   - Use traits to abstract platform differences

3. **Assembly code:**
   - Keep assembly minimal and well-documented
   - Prefer Rust with inline asm when possible
   - Document register usage and calling conventions

4. **Interrupt handling:**
   - Always platform / arch specific best practices
   - Interrupts currently disabled during critical sections

5. **Testing:**
   - **Write unit tests for all kernel modules** (see `kernel/src/scheduler/mlfq_strategy.rs` as example)
   - Unit tests should be comprehensive and test edge cases
   - Use `#[cfg(test)]` modules within each file
   - Integration testing: Use dummy tasks for scheduler/task testing
   - Use longer delays to observe task switching behavior in QEMU
   - Verify behavior in QEMU and eventually real hardware

6. **Warnings are errors:** `[workspace.lints.rust]` in the root `Cargo.toml` denies the unused family, implicit unsafe ops in `unsafe fn`, private-in-public interfaces, irrefutable `let-else`, function-item casts, and deprecated APIs; every member crate inherits it. Every build surface (both arches, apps, host unit tests) must end at zero warnings — the only expected diagnostic is the toolchain's own `core` future-incompat report, an artifact of `-Zbuild-std` compiling rust-src and not this repository's code. Deliberate exceptions use `#[expect(lint, reason = "...")]`, never workspace-wide allowances, and never without the maintainer's explicit approval for that specific case — the approved font palette in `arch/x86_64/src/terminal_fonts.rs` is such an exception and is not a precedent.

### 486 baseline (x86_32)

The x86_32 port runs on 486-class CPUs and must stay that way — the invariant, not the bug, is the default:

- **The pinned feature strings are the floor.** `arch/x86_32/rosx-i486.json` and `rosx-i486-user.json` compile against `-mmx,-sse,-sse2,-cmov,-fxsr,-cx8`; never widen them. `-cx8` pins `cmpxchg8b` out to keep the 486SX door open. Note: `RUSTFLAGS` in the environment *replaces* the `[target.rosx-*-user]` rustflags in `.cargo/config.toml` (which carry the `linker.ld` script) — never build user ELFs with an exported `RUSTFLAGS`.
- **No floating point on any 32-bit target** — kernel, usrlib, or apps. `swap_context` saves GPRs only, so any FP state would silently corrupt across task switches, and a 486SX has no FPU at all.
- **No `cpuid`-probed features** — `cpuid` itself is 586-era; the port runs flat protected mode with 386-era mechanism only (no paging, no CR4 bits).
- **Keep atomics ≤ 32 bits** on 32-bit paths; an `AtomicU64` there needs `cmpxchg8b`.
- **The machine guards:** `cargo xtask check-isa x86_32` audits the kernel and every user ELF against a 486 mnemonic allow-list, `cargo xtask check-isa x86_32 --stages` audits the pinned GRUB Legacy stage binaries against a post-486 denylist (CI runs both in `build-x86_32`), and every x86_32 QEMU session boots `-cpu 486`. A regression must fail one of these, never a board.
- **The two byte-gated allow-list exceptions:** `tzcnt` passes only in its legacy `f3`-prefixed form (executes as 386 `BSF`), and `ud2` passes only with the exact reserved `0f 0b` encoding — that opcode raises #UD identically on 486 silicon (Intel chose it for exactly that reason), and the compiler emits it only as a trap after a no-return panic path. Any other bytes decoding as `tzcnt`/`ud2` still fail the audit.
- Provenance: `plans/x86_32-i486-plan.md` and the evidence log `plans/x86_32-i486-impl.md`.

### Code Style

**Memory Safety & Ownership:**
- **CRITICAL:** Use Rust's safe memory management and ownership control as much as possible
- **`unsafe` usage:** Only use `unsafe` when absolutely unavoidable (e.g., raw hardware access, inline assembly)
- When `unsafe` is required:
  - Document WHY it's necessary
  - Document what invariants must be maintained
  - Keep `unsafe` blocks as small as possible
  - Provide safe wrappers around unsafe operations

**Hardware Abstraction:**
- **All hardware-specific routines MUST be abstracted from the kernel**
- Use Hardware Abstraction Layer (HAL) pattern - see the `Cpu` trait as the reference example
- Kernel code in `kernel/` should be completely platform-agnostic
- Platform-specific implementations go in `arch/[platform]/`
- Key HAL traits: `Cpu` (`kernel/src/platform/cpu.rs`), `ElfArch` (`kernel/src/platform/elf.rs`), `PortIo` (`kernel/src/platform/portio.rs`)

**No Test-Only Production Surface:**
- Every function, method, or constructor in production code must have at least one non-test caller; `pub(crate)` visibility is not an excuse
- Test scaffolding (inspectors, fixtures, wake/task helpers) lives inside the `#[cfg(test)]` module of the unit under test, never in production code paths
- If a test needs access beyond the production interface, reshape the test or give the function a real caller - never widen the production surface to serve a test
- Symmetric rule: production API without any caller (test or otherwise) is dead weight - wire a real user or delete it

**Pluggable Architecture:**
- **Scheduling is pluggable (implemented):** the strategy is a `SchedulingAlgorithm` (MLFQ + FIFO) chosen at boot via `scheduler_factory` in `KConfig` (`kernel/src/kconfig.rs`).
- **Memory is NOT pluggable (yet):** a single static `global_allocator` (`MemoryManager` → `FreeListAllocator`) is active; a `BitmapChunkAllocator` exists but is not wired in. Making the allocator selectable is a stated **low-priority goal**.
- Prefer trait-based design so subsystems can be swapped at boot time, not compile time.

**Architecture Principles (who owns what):**
- **Domain knowledge is owned by the allocating subsystem.** A kernel resource's identity, layout, and validity rules live with the subsystem that allocates and validates it; OS policy must not live in infrastructure crates.
- **The kernel is the single authority on invariants.** Validation happens once, kernel-side; user space carries opaque tokens and neither re-checks nor duplicates kernel invariants.
- **The shared treaty holds the contract, not the mechanism.** `system` contains numbers and layouts only: a type that describes *how the kernel works* is in the wrong crate; a type that describes *what the boundary contract says* belongs in `system`.
- **Translation lives at the boundaries.** The kernel edge encodes domain→wire and the user edge decodes wire→user-view; the shared crate cannot own either translation — the dependency arrows forbid it.
- **User-visible APIs expose no kernel internals.** Debug aids are kernel-side and never part of the ABI.

These principles are the regression guard: the very inversion this rework removed survived many runs because no document stated who owns handle semantics, while the HAL rule — written down — has been obeyed throughout.

**Kernel Subsystems (current invariants):**
**Subsystem design records:** `docs/architecture/` holds one file per subsystem (scheduling-and-interrupts, ipc-and-futures, exit-status-and-ownership, memory, boot-plan, disk-and-blockd, network-and-ip, vfs, fs-engines, kstatsd-and-rom, input, real-hardware-x86_32).

**Documentation & Comments:**
- **CRITICAL: No comments, ever** — the code is the source of truth; make it self-explanatory
- Self-explain through:
  - Clear function names
  - Descriptive variable names
  - Well-structured logic
  - Type signatures that document intent
- Exception: Assembly code must have comments (register usage, calling conventions)

**General Style:**
- Follow standard Rust conventions
- Keep functions focused and modular
- Prefer type safety over raw pointers/integers where possible
- Use descriptive names for types, functions, and variables

---

### Build Issues?
- Toolchain: `rust-toolchain.toml` pins **nightly** with the `rust-src` component (`rustup component add rust-src`).
- Custom targets: `arch/x86_64/rosx.json` (+ `rosx-user.json`), `arch/x86_32/rosx-i486.json` (+ `rosx-i486-user.json`).
- No bootimage — the disk image comes from `arch/x86-runner` (the `bootloader` crate). To build it standalone: `cargo run --manifest-path arch/x86-runner/Cargo.toml -- <kernel-binary> x86_64 --no-run`.

### GitHub CLI?
- `gh pr edit` on `rghisi/rosx` fails here with a Projects (classic) deprecation GraphQL error (`repository.pullRequest.projectCards`) even when touching no project field. Update PRs through the REST API instead: `gh api -X PATCH repos/rghisi/rosx/pulls/<n> -f title="..." -F body=@<file>`.

---

## Notes for AI Assistants

### ⚠️ MOST IMPORTANT - Work Incrementally ⚠️

**MANDATORY: Work in very small steps - NEVER write code without explanation and approval**

This is the MOST CRITICAL guideline for working on RosX:

1. **Propose changes BEFORE writing code**
   - Explain what you plan to do
   - Explain WHY you're doing it
   - Describe the approach you'll take
   - Ask for feedback/approval

2. **Work in tiny increments**
   - One small change at a time
   - One function, one file, one concept
   - Make it work, verify, then move to next step
   - Before moving to the next step, ask for confirmation, create a commit message and confirm
   - The commit message should not contain any prefix

3. **Never batch multiple changes**
   - Don't implement several features at once
   - Don't modify multiple files without discussing each
   - Don't assume the next step - always ask

4. **Continuous communication**
   - Explain your reasoning
   - Discuss trade-offs
   - Share alternative approaches
   - Wait for user feedback before proceeding

**This is a learning project - the journey and understanding are as important as the destination.**

---

### Other Critical Guidelines

- **CRITICAL: Minimize `unsafe` usage** - Use Rust's safe abstractions whenever possible; `unsafe` only when truly unavoidable
- **CRITICAL: Hardware abstraction required** - All platform-specific code must go through HAL traits (like `Cpu`), never directly in kernel code
- **CRITICAL: kernel/ is `no_std`** - std is forbidden; dependencies are limited to the workspace crate `system`. No other third-party crates.
- **CRITICAL: No comments, ever** - Write self-explanatory code with clear names instead
- **CRITICAL: Write unit tests** - All kernel modules should have comprehensive unit tests (see `kernel/src/scheduler/mlfq_strategy.rs` as a reference)
- **CRITICAL: No test-only functions in production code** - production functions need non-test callers; test helpers belong in `#[cfg(test)]` modules, and production APIs need real users
- Multi-platform support is a goal - keep kernel code portable and platform-agnostic
- Build and test after significant changes (`cargo test -p kernel`)
