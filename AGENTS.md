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
  - **ARM** (planned soon - high priority)
  - **RISC-V** (planned soon - high priority)
  - **m68k** (planned soon - high priority)
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
│   ├── arm/                # ARM port (planned soon - high priority)
│   ├── riscv/              # RISC-V port (planned soon - high priority)
│   └── m68k/               # m68k port (planned soon - high priority)
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

## Current State

### What Works
- Basic task switching using `ret`-based context switching
- Idle task execution when no tasks are ready
- Task queue management (ready queue)
- Keyboard interrupt handling (triggers task switching)
- PIC (8259) configuration and interrupt masking
- Bootloader integration (bootimage)

### Known Issues
- **Task finalization handling problem:** Tasks that complete have issues with the finalization process (marking as terminated and yielding back to MainThread)

### Active Work
**PRIMARY FOCUS:** Get basic preemptive scheduler working

**Major Refactoring in Progress:** Transitioning from `ret`-based to interrupt-driven context switching using `iretq`

**Goal:** Enable true preemptive multitasking where:
- All context switches go through interrupts
- Voluntary yields use software interrupt (INT 0x30)
- Hardware interrupts (timer, keyboard) can preempt tasks
- Unified interrupt stack frame for all switches
- **Timer-based preemption** for fair scheduling

**Current Phase:** Planning complete, ready to begin implementation
**Reference:** See `INTERRUPT_DRIVEN_CONTEXT_SWITCH_REFACTORING.md` for detailed 15-step plan

**Priority:** Complete the interrupt-driven refactoring and get timer-based preemption working before adding other features

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
- **Current:** QEMU for testing (fast iteration)
- **Future Goal:** Run on real x86_64 hardware
- Bootloader supports both QEMU and real hardware deployment

### Custom Target
- Target spec: `arch/x86_64/rosx.json`
- Bare metal (no_std)
- Builds core, alloc, compiler_builtins from source

---

## Architecture Details

### x86_64 Specifics

**Interrupt Setup:**
- IDT (Interrupt Descriptor Table) configured
- PICs remapped: PIC1 @ 0x20, PIC2 @ 0x28
- Keyboard interrupt (IRQ1) enabled and triggers context switch
- Timer interrupt (IRQ0) planned for future preemptive scheduling

**Context Switching (Current):**
- Uses `swap_context` assembly function
- Saves/restores 15 general-purpose registers
- Uses `ret` instruction to jump to tasks
- Task initialization creates stack frame for `ret`-based entry

**Context Switching (Target):**
- Interrupt-driven using `iretq`
- Software interrupt 0x30 for voluntary yields
- Hardware interrupts for preemption
- Unified interrupt stack frame (144 bytes)

**Memory Layout:**
- Stack grows downward
- 16-byte stack alignment required
- Task stacks initialized with proper alignment

---

## Key Components

### Task System

**Task Structure** (`kernel/src/task.rs`)
- Stack pointer
- State (New, Ready, Running, Terminated)
- Entry point and parameters
- Task wrapper function for cleanup

**Scheduler** (`kernel/src/simple_scheduler.rs`)
- **Design Goal:** Pluggable during kernel configuration/bootstrapping
- Current implementation: Round-robin scheduling
- Ready queue (VecDeque)
- Idle task when no ready tasks available
- **Future:** Multiple scheduler implementations selectable at boot time

**Main Thread** (`kernel/src/main_thread.rs`)
- Core scheduling loop
- Manages task queue
- Re-queues yielded tasks, discards terminated tasks
- Enables interrupts after initialization

### Interrupt Handling

**Current Handlers** (`arch/x86_64/src/interrupts.rs`)
- Keyboard interrupt: Reads scancode, sends EOI, calls `task_yield()`
- Uses `x86-interrupt` ABI from x86_64 crate

**Planned Handlers:**
- Software interrupt 0x30 for voluntary yields
- Timer interrupt for preemptive scheduling
- Additional hardware interrupts as needed

---

## Dependencies

### Key Crates
- `x86_64 = "0.15"` - x86_64 structures and intrinsics
- `pic8259 = "0.11"` - PIC (Programmable Interrupt Controller) driver
- `bootimage = "0.11"` - Bootloader integration

### Standard Library
- `#![no_std]` - Bare metal, no standard library
- `core` and `alloc` built from source
- Custom panic handler required

### Dependency Policy
- **CRITICAL: The `kernel/` crate must NOT depend on ANY external crates**
- Only base Rust crates allowed in `kernel/`: `core`, `alloc`, `compiler_builtins`
- Architecture crates (`arch/*`) may use platform-specific crates (e.g., `x86_64`, `pic8259`)
- Keep dependencies minimal across the entire project

---

## Development Guidelines

### When Working on RosX

1. **Check current state first:**
   - Read git status and recent commits
   - Check DEVELOPMENT_LOG.md for recent changes
   - Review INTERRUPT_DRIVEN_CONTEXT_SWITCH_REFACTORING.md if working on context switching

2. **Understand the architecture layer:**
   - `kernel/` should remain platform-agnostic
   - Architecture-specific code goes in `arch/[platform]/`
   - Use traits to abstract platform differences

3. **Assembly code:**
   - Keep assembly minimal and well-documented
   - Prefer Rust with inline asm when possible
   - Document register usage and calling conventions

4. **Interrupt handling:**
   - Always send EOI to PIC when handling hardware interrupts
   - Be mindful of interrupt masking
   - Interrupts currently disabled during critical sections

5. **Task management:**
   - Tasks must properly mark themselves as terminated
   - Stack management is manual - be careful with alignment
   - Task wrapper ensures cleanup

6. **Testing:**
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

## Common Tasks

### Adding a New Interrupt Handler
1. Define interrupt number in `arch/x86_64/src/interrupts.rs`
2. Create handler function with `extern "x86-interrupt"` ABI
3. Register in IDT during `init()`
4. Unmask interrupt in PIC if hardware interrupt
5. Test with appropriate trigger

### Adding a New Task
1. Implement `Runnable` trait or use `FunctionTask`
2. Initialize task stack with `initialize_task_for_swap`
3. Add to scheduler's ready queue
4. Task should call `task_yield()` to cooperate

### Debugging Context Switch Issues
1. Check stack alignment (must be 16-byte aligned)
2. Verify all registers are saved/restored
3. Ensure stack pointer is valid after switch
4. Check task state transitions
5. Look for stack corruption

---

## Future Roadmap

### Near Term (Next Steps)
1. Complete interrupt-driven context switching refactoring
2. Fix task finalization handling issue
3. Add timer interrupt for true preemptive multitasking
4. Implement proper exception handlers (page fault, etc.)
5. Expand unit test coverage for existing kernel modules

### Medium Term
- **Pluggable scheduler system** - Enable multiple scheduler implementations selectable at boot
- **Pluggable memory management** - Replace pre-baked memory manager with configurable implementations
  - Currently using pre-baked memory manager
  - Goal: Multiple memory managers that can be selected during bootstrapping
- Process isolation (user mode)
- System calls interface
- Basic device drivers (serial, disk)

### Long Term (Multi-Platform Ports)
- **ARM port** (high priority - planned soon)
- **RISC-V port** (high priority - planned soon)
- **m68k port** (high priority - planned soon)
- Microcontroller support
- Networking stack (TCP/IP)
- File system
- User-space applications (web server, etc.)

---

## Quick Reference

### Important Files to Check
- `arch/x86_64/src/context_switching.S` - Core context switch assembly
- `arch/x86_64/src/interrupts.rs` - All interrupt handlers
- `kernel/src/kernel.rs` - Kernel initialization and task_yield API
- `kernel/src/main_thread.rs` - Main scheduling loop
- `DEVELOPMENT_LOG.md` - Recent development history
- `INTERRUPT_DRIVEN_CONTEXT_SWITCH_REFACTORING.md` - Current refactoring plan

### Git Branch Strategy
- `working-with-claude` - Current working branch
- No main branch set yet (to be configured)

### Build Issues?
- Check that custom target exists: `arch/x86_64/rosx.json`
- Verify rust-src component: `rustup component add rust-src`
- Bootimage installed: `cargo install bootimage`

---

## Questions to Ask When Starting Work

1. **What's the current git status?** - Check for uncommitted changes
2. **What was the last thing worked on?** - Read DEVELOPMENT_LOG.md
3. **Are there any known blockers?** - Check "Current Issues" section in logs
4. **Which phase of refactoring are we in?** - Check INTERRUPT_DRIVEN_CONTEXT_SWITCH_REFACTORING.md
5. **Is the code buildable?** - Try `cargo build` before making changes

---

## Emergency Recovery

If the codebase is in an unknown state:

1. **Check git log:** `git log --oneline -10`
2. **Check working tree:** `git status`
3. **Read development log:** `cat DEVELOPMENT_LOG.md` (focus on last session)
4. **Try to build:** `cd arch/x86_64 && cargo build`
5. **Check for uncommitted experiments:** `git diff`

Last known good commit: `ac0c8f6` - "Back to working state, next is preemptive task scheduling"

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
- Update DEVELOPMENT_LOG.md when completing major features or sessions
- When adding subsystems (schedulers, memory managers, etc.), design them as pluggable trait implementations