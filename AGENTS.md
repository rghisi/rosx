# RosX - Agent Context Document

> **Last Updated:** 2025-10-12
> **Purpose:** Quick context recovery for AI coding assistants working on RosX

---

## Project Overview

**RosX** is a learning/hobby OS project written in Rust, aiming to be a multi-platform operating system capable of running on diverse hardware - from microcontrollers to m68k computers to x86_64 systems. The goal is to create a multi-purpose OS that can run network applications such as web servers.

### Key Characteristics
- **Language:** Rust (with minimal assembly for architecture-specific code)
- **Target Platforms:**
  - **x86_64** (current - chosen for readily available infrastructure)
  - **x86_32** (next - to run on older PCs)
  - **ARM** (future)
  - **RISC-V** (future)
  - **m68k** (future)
  - Microcontrollers (future)
- **Scope:** Full-featured OS with networking, task scheduling, interrupt handling
- **Project Type:** Learning/hobby project with practical goals

**Platform Strategy:** x86_64 is NOT the primary focus - it's just the starting point due to tooling availability. Multi-platform support is a core design goal, not an afterthought. All kernel code must remain portable.

---

## Project Structure

```
rosx/
├── arch/                    # Architecture-specific implementations
│   ├── x86_64/             # Current implementation (bootstrap platform)
│   │   ├── src/
│   │   │   ├── context_switching.S      # Assembly context switch code
│   │   │   ├── process_initialization.S # Task initialization assembly
│   │   │   ├── interrupts.rs            # IDT, PIC, interrupt handlers
│   │   │   ├── cpu.rs                   # CPU trait implementation
│   │   │   └── main.rs                  # Architecture entry point
│   │   ├── .cargo/config.toml          # Build config (custom target)
│   │   └── Cargo.toml
│   ├── arm/                # ARM port (future)
│   ├── riscv/              # RISC-V port (future)
│   └── m68k/               # m68k port (future)
│
├── kernel/                 # Platform-agnostic kernel code
│   ├── src/
│   │   ├── kernel.rs       # Main kernel struct, task_yield()
│   │   ├── task.rs         # Task structure and management
│   │   ├── main_thread.rs  # Main scheduler thread
│   │   ├── scheduler.rs    # Scheduler trait
│   │   ├── simple_scheduler.rs  # Basic round-robin scheduler
│   │   ├── function_task.rs     # Function-based tasks
│   │   └── cpu.rs          # CPU trait definition
│   └── Cargo.toml
│
├── docs/                   # Documentation
├── DEVELOPMENT_LOG.md      # Session-based development history
├── INTERRUPT_DRIVEN_CONTEXT_SWITCH_REFACTORING.md  # Current refactoring plan
└── Cargo.toml             # Workspace configuration
```

---

## Build & Run

### Build Command
```bash
cd arch/x86_64
cargo build
```

### Run with Bootloader
```bash
cd arch/x86_64
cargo run  # Uses bootimage runner configured in .cargo/config.toml
```

**Testing Workflow:**
- **Manual:** QEMU run for manual testing (fast iteration)
- **Automated:** Run unit tests in host arch

### Custom Target
- Target spec: `arch/x86_64/rosx.json`
- Bare metal (no_std)
- Builds core, alloc, compiler_builtins from source

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
- Check that custom target exists: `arch/x86_64/rosx.json`
- Verify rust-src component: `rustup component add rust-src`
- Bootimage installed: `cargo install bootimage`

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