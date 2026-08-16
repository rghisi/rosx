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
     [workspace members]  shell, dummy
     [excluded ELF apps]  hello_elf, random_gen_server, snake, tetris, conway
```

---

## Build & Run

Canonical recipes: `.github/workflows/ci.yml` (build + test) and `build-artifacts.yml` (disk images).

### x86_64
```bash
cd arch/x86_64
cargo build              # kernel -> target/rosx/debug/rosx   (add --release to match CI)
cargo run                # run.sh -> arch/x86_64-runner -> BiosBoot disk image -> QEMU
```
- `cargo run` uses the **custom runner** (`.cargo/config.toml` → `./run.sh`), which invokes `arch/x86_64-runner` to make a BIOS disk image with the `bootloader` crate's `BiosBoot`, then starts `qemu-system-x86_64`. **Not bootimage.**
- Target spec: `arch/x86_64/rosx.json` — bare-metal `no_std`, `build-std` = core/alloc/compiler_builtins.
- User-space ELF apps (hello_elf, random_gen_server, snake, tetris, conway) build separately with `--target rosx-user.json` (PIC/PIE), then get embedded into the kernel via `include_bytes!`.

### x86_32
```bash
cd arch/x86_32
cargo build              # kernel (Multiboot2, boot.S)
bash build-image.sh      # GRUB bootable image (needs grub-mkrescue, xorriso, mtools)
```

### Unit tests (run on host)
```bash
cargo test -p kernel       # ~149 tests: scheduler, memory, elf, ipc, future
cargo test -p collections  # generational_arena
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
   - **Write unit tests for all kernel modules** (see `kernel/src/simple_scheduler.rs` as example)
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
- Use Hardware Abstraction Layer (HAL) pattern - see `Cpu` trait as example
- Kernel code in `kernel/` should be completely platform-agnostic
- Platform-specific implementations go in `arch/[platform]/`
- Use traits to define hardware interfaces (like `Cpu`, `Scheduler`, `Runnable`)

**Pluggable Architecture:**
- **Schedulers must be pluggable** - Configurable during kernel bootstrapping, not hardcoded
- **Memory managers must be pluggable** - Selectable during bootstrapping phase
- Use trait-based design to allow multiple implementations
- Configuration happens at boot time, not compile time (where feasible)
- Goal: Easy experimentation with different strategies for different use cases

**Documentation & Comments:**
- **CRITICAL: Minimal documentation - code is the source of truth**
- **CRITICAL: NO code comments unless explicitly requested**
- Code should be self-explanatory through:
  - Clear function names
  - Descriptive variable names
  - Well-structured logic
  - Type signatures that document intent
- Exception: Assembly code should have comments explaining register usage and calling conventions
- Focus on writing readable code rather than explaining it with comments

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
- **CRITICAL: Hardware abstraction required** - All platform-specific code must go through HAL traits (like `Cpu` trait), never directly in kernel code
- **CRITICAL: Pluggable architecture** - Schedulers and memory managers must be configurable at bootstrap, not hardcoded
- **CRITICAL: Zero external dependencies in kernel/** - Only `core`, `alloc`, `compiler_builtins` allowed
- **CRITICAL: No code comments unless requested** - Write self-explanatory code with clear names instead
- **CRITICAL: Write unit tests** - All kernel modules should have comprehensive unit tests (see `simple_scheduler.rs` example)
- Multi-platform support is a goal - keep kernel code portable and platform-agnostic
- Assembly should be minimal and well-documented (exception to no-comments rule)
- The project is in active refactoring - check the refactoring doc before context switching work
- Task finalization is currently broken - don't assume it works
- Build and test after significant changes
- When adding subsystems (schedulers, memory managers, etc.), design them as pluggable trait implementations
