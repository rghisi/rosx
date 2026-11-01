# RosX - Agent Context Document

> **Last Updated:** 2026-11-01
> **Purpose:** Compact, always-accurate context for AI coding agents working on RosX.

---

## Project Overview

**RosX** is a learning/hobby OS project written in Rust, aiming to be a multi-platform operating system capable of running on diverse hardware - from microcontrollers to m68k computers to x86_64 systems. The goal is to create a multi-purpose OS that can run network applications such as web servers.

### Key Characteristics
- **Language:** Rust (with minimal assembly for architecture-specific code)
- **Target Platforms (status):**
  - **x86_64** — bootstrap platform, complete.
  - **x86_32** — complete (Multiboot2/GRUB, `arch/x86_32/`).
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
│    elf/ ipc/ (mailbox, data) future/ syscall/ kstatsd task*  platform/ (cpu, elf, interrupt, output, portio HAL traits + per-task port grants)  kconfig  rom/
├── system/      # ABI treaty: syscall numbers, wire layout, ROM image layout, error numbers
├── usrlib/      # User-space libc: allocator (binned slabs, global default), syscall, out, arch/{x86_64,x86_32}
├── protocol/                # Service RPC treaties, one sub-crate each (workspace glob protocol/*)
│    vfs/         # package vfs-protocol: wire codec + typed client
│    input/       # package input-protocol: wire codec
│    ata/         # package ata-protocol: wire codec + typed client
├── arch/
│    x86_64/        # entry, cpu, interrupts, framebuffer, terminal_fonts, *.S (global_asm)
│    x86_32/        # entry, cpu, interrupts, boot.S (Multiboot2)
│    x86-generic/   # shared x86-family mechanism: port executor, VGA text buffer, debug console, ANSI parser
│    x86-runner/    # workspace member: kernel → boot medium + QEMU launcher for both ports (NOT bootimage)
└── apps/
     [workspace members]  shell, dummy, pi, randomd, echo, bulk, pong, ping, mem, vfsd, vfs, snake, tetris, conway, threads, waitset, stress, inputd, ps2d, atad, ata
```

---

## Build & Run

Canonical recipes: `.github/workflows/ci.yml` (build + test) and `build-artifacts.yml` (disk images).

### x86_64
```bash
cargo xtask build              # ELF apps + ROM image + kernel + disk image
cargo xtask apps               # ELF apps only
cargo xtask rom                # ROM image from built apps
cargo xtask test               # all host unit tests
cargo build -p rosx            # kernel only (from arch/x86_64 directory or with target)
cargo run -p rosx              # run.sh -> arch/x86-runner -> BiosBoot disk image -> QEMU
```
- `cargo run` uses the **custom runner** (`.cargo/config.toml` → `./run.sh`), which invokes `arch/x86-runner` to make a BIOS disk image with the `bootloader` crate's `BiosBoot`, then starts `qemu-system-x86_64`. **Not bootimage.**
- Target spec: `arch/x86_64/rosx.json` — bare-metal `no_std`, `build-std` = core/alloc/compiler_builtins.
- User-space ELF apps (pi, randomd, snake, tetris, conway) are workspace members; build with `cargo xtask apps` or `cargo xtask build`.
- The shell is a user ELF: each arch port embeds `target/rosx-user/release/shell` with `include_bytes!` and boots it via `new_elf_task`. It keeps five built-ins (`ls clear rose sleep random`) and resolves every other prompt name through VFS, so ROM/RAM files execute by name with arguments (`rose> snake fast`).
- `cargo build --workspace` is intentionally unsupported: bare-metal bins and PIE apps cannot compile for the host.

### x86_32
```bash
cargo xtask build x86_32       # ELF apps + ROM image + kernel (Multiboot2, boot.S)
cargo xtask run x86_32         # boot QEMU (-kernel, qemu-system-i386) to the shell prompt
cargo xtask image x86_32       # GRUB bootable ISO (needs grub-mkrescue, xorriso)
cargo build -p rosx-x86_32     # kernel only (from arch/x86_32/ — its .cargo/config.toml supplies the rosx-i686 custom target)
```
- The kernel embeds the shell ELF and the ROM image that `cargo xtask build x86_32` produces under `target/rosx-i686-user/release/`; `arch/x86_32/build.rs` asserts both are present before compiling.
- The boot roster matches x86_64 (randomd, echo, bulk, pong, vfsd, inputd, ps2d, atad with ATA port grants, then the shell), so programs run by name from the prompt and the disk stack works there.
- Integration tests run on x86_32 with `ROSX_ARCH=x86_32 cargo test -p tests-integration -- --test-threads=1`; the harness boots the kernel ELF directly and synthesizes the two-drive IDE topology for the ata suites. `ipc_vfs` and `ata_env_spike` stay x86_64-only; `kstats_mem` runs unguarded on both arches (the wake-path race is fixed — `plans/x86_32-wake-race.md`).
- 32-bit targets use a 16 KiB task stack (`STACK_WORDS` in `kernel/src/task_stack.rs`): the deepest debug syscall chain with a nested IRQ frame overflows the 8 KiB x86_64 stack and corrupts adjacent kernel heap blocks.

### Unit tests (run on host)
```bash
cargo xtask test               # all workspace unit tests
cargo test                     # default-members only
cargo test -p kernel -- --test-threads=1   # scheduler, memory, elf, ipc, future
```

### Integration tests (run on host against QEMU)
```bash
cargo test -p tests-integration -- --test-threads=1                      # x86_64
ROSX_ARCH=x86_32 cargo test -p tests-integration -- --test-threads=1      # x86_32
```

### xtask commands
```bash
cargo xtask build [arch] [--debug]  # ELF apps + ROM + kernel + boot artifact (arch: x86_64 default, or x86_32)
cargo xtask apps [arch]             # Build ELF apps only
cargo xtask rom [arch]              # Pack the ROM image from the built user ELFs
cargo xtask run [arch] [--debug]    # Build and launch QEMU to the shell prompt
cargo xtask image [arch] [--debug]  # flashable artifact: x86_64 BiosBoot disk image, x86_32 GRUB ISO (needs grub-mkrescue, xorriso)
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
- `future/` + `ipc/` — **notification-driven** futures (time, task-completion, depth-one waitset aggregates) and a mailbox IPC manager (bind/connect/send/receive); kernel-originated event delivery (`deliver_to_service`) wakes accepts on bound services; a waitset completes when any attached leaf future does, woken by a child-to-waitset cascade inside `FutureRegistry::notify`; no polling on the preemption cycle.
- Interrupt discipline — kernel code runs with IF=0 except idle `hlt` and the user-return boundaries (`iretd`/`SFMASK`); the scheduler, future registry, and mailbox manager mutations are `IrqGuard` sections (`kernel/src/irq_guard.rs`) whose re-entry tripwire prints `[IRQGUARD]` and must stay silent.
- `ipc/` data family — messages signal; data rides the connection out-of-band via write/read/release tokens; the kernel never interprets message payloads.
- Input architecture — policy lives in the user-space `INPUTD` daemon (`inputd`), device semantics in driver daemons (`ps2d`); the kernel delivers opaque event frames only.
- Disk architecture — ATA command semantics live in the user-space `ATAD` daemon (`atad`) over `ata-protocol`; the kernel contributes only the mediated port capability (`portio` grants). v1 polls; IRQ14 delivery is the v2 event path.
- Kernel-hosted IPC services — `kstatsd` is the first: kernel code runs a service loop as a `FunctionTask` (launched by each arch at boot), calling `IpcManager` directly and blocking on futures via `Kernel::wait`. Metrics stay domain-owned (`MemoryManager::stats`); the wire contract lives only in `system::kstats`; user-space clients (`mem`) own rendering.
- `elf/` — loads standalone user-space ELF binaries into tasks (`new_elf_task`).
- `task/` + `task_manager/` — task lifecycle, context switch, preemption.
- **Memory ownership = loaded-image groups** — user blocks are stamped `BlockOwner::Group`: `LoadElf` (and boot loads) starts a fresh group, `Spawn` siblings join the caller's group (they share that image's statics), and the kernel reclaims a group's memory when its last live member terminates.
- `rom/` — boot-registered immutable ROM image exposed via the opaque `RomRead` syscall; the kernel stays VFS-agnostic, `vfsd` owns mount semantics. The ROM carries the app ELFs (not the shell); its roster names stay underscore-free — a historical constraint from the era when the kernel keyboard dropped shift state, with the rename tracked in `docs/todo.md`.

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
- Custom targets: `arch/x86_64/rosx.json` (+ `rosx-user.json`), `arch/x86_32/rosx-i686.json` (+ `rosx-i686-user.json`).
- No bootimage — the disk image comes from `arch/x86-runner` (the `bootloader` crate). To build it standalone: `cargo run --manifest-path arch/x86-runner/Cargo.toml -- <kernel-binary> x86_64 --no-run`.

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
