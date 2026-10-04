# RosX - Agent Context Document

> **Last Updated:** 2026-08-16
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
│    elf/ ipc/ (mailbox) future/ syscall/ keyboard/ task*  cpu (HAL trait)  kconfig
├── system/      # Abstractions: Future, syscall numbers, IPC types
├── collections/ # generational_arena
├── usrlib/      # User-space libc: syscall, out, arch/{x86_64,x86_32}
├── arch/
│    x86_64/        # entry, cpu, interrupts, framebuffer, terminal_fonts, *.S (global_asm)
│    x86_32/        # entry, cpu, interrupts, boot.S (Multiboot2)
│    x86_64-runner/ # standalone: BiosBoot → QEMU (NOT bootimage)
└── apps/
     [workspace members]  shell, dummy, hello_elf, random_gen_server, snake, tetris, conway, test_suite
```

---

## Build & Run

Canonical recipes: `.github/workflows/ci.yml` (build + test) and `build-artifacts.yml` (disk images).

### x86_64
```bash
cargo xtask build              # ELF apps + kernel + disk image
cargo xtask apps               # ELF apps only
cargo xtask test               # all host unit tests
cargo build -p rosx            # kernel only (from arch/x86_64 directory or with target)
cargo run -p rosx              # run.sh -> arch/x86_64-runner -> BiosBoot disk image -> QEMU
```
- `cargo run` uses the **custom runner** (`.cargo/config.toml` → `./run.sh`), which invokes `arch/x86_64-runner` to make a BIOS disk image with the `bootloader` crate's `BiosBoot`, then starts `qemu-system-x86_64`. **Not bootimage.**
- Target spec: `arch/x86_64/rosx.json` — bare-metal `no_std`, `build-std` = core/alloc/compiler_builtins.
- User-space ELF apps (hello_elf, random_gen_server, snake, tetris, conway) are workspace members; build with `cargo xtask apps` or `cargo xtask build`.
- `cargo build --workspace` is intentionally unsupported: bare-metal bins and PIE apps cannot compile for the host.

### x86_32
```bash
cargo build -p rosx-i686       # kernel (Multiboot2, boot.S)
bash arch/x86_32/build-image.sh   # GRUB bootable image (needs grub-mkrescue, xorriso, mtools)
```
- x86_32 builds are untouched by xtask. Build user apps for x86_32 with the explicit `-Z` flags shown in CI.

### Unit tests (run on host)
```bash
cargo xtask test               # all workspace unit tests
cargo test                     # default-members only
cargo test -p collections      # generational_arena
cargo test -p kernel -- --test-threads=1   # scheduler, memory, elf, ipc, future
```

### xtask commands
```bash
cargo xtask build [--debug]    # Build ELF apps + x86_64 kernel + disk image
cargo xtask apps               # Build ELF apps only
cargo xtask test [--integration] # Run unit tests; --integration is reserved
```

## Development Guidelines

### When Working on RosX

1. **Check current state first:**
   - Read git status and recent commits

2. **Understand the architecture layer:**
   - `kernel/` should remain platform-agnostic
   - Architecture-specific code goes in `arch/[platform]/`
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
- Key HAL traits: `Cpu` (`kernel/src/cpu.rs`), `ElfArch` (`kernel/src/elf/arch.rs`)

**Pluggable Architecture:**
- **Scheduling is pluggable (implemented):** the strategy is a `SchedulingAlgorithm` (MLFQ + FIFO) chosen at boot via `scheduler_factory` in `KConfig` (`kernel/src/kconfig.rs`).
- **Memory is NOT pluggable (yet):** a single static `global_allocator` (`MemoryManager` → `FreeListAllocator`) is active; a `BitmapChunkAllocator` exists but is not wired in. Making the allocator selectable is a stated **low-priority goal**.
- Prefer trait-based design so subsystems can be swapped at boot time, not compile time.

**Kernel Subsystems (current invariants):**
- `future/` + `ipc/` — **notification-driven** futures (time, task-completion, keyboard) and a mailbox IPC manager (bind/connect/send/receive); no polling on the preemption cycle.
- `elf/` — loads standalone user-space ELF binaries into tasks (`new_elf_task`).
- `task/` + `task_manager/` — task lifecycle, context switch, preemption.

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
- No bootimage — the disk image comes from `arch/x86_64-runner` (the `bootloader` crate). To build it standalone: `cargo run --manifest-path arch/x86_64-runner/Cargo.toml -- <kernel-binary> x86_64 --no-run`.

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
- **CRITICAL: kernel/ is `no_std`** - std is forbidden; dependencies are limited to the workspace crates `collections` + `system` and `lazy_static`. No other third-party crates.
- **CRITICAL: No comments, ever** - Write self-explanatory code with clear names instead
- **CRITICAL: Write unit tests** - All kernel modules should have comprehensive unit tests (see `kernel/src/scheduler/mlfq_strategy.rs` as a reference)
- Multi-platform support is a goal - keep kernel code portable and platform-agnostic
- Build and test after significant changes (`cargo test -p kernel`)
