# RosX

RosX is a multi-platform operating system written in Rust from scratch. It is designed with a focus on portability, a microkernel-inspired architecture, and networking capabilities. A key design goal is to not require an MMU, enabling it to run on smaller and simpler devices.

## Current Status

**RosX is currently under active development.** The x86_64 platform is functional with basic multitasking, while x86_32 support has been implemented. The project serves as both a learning exercise and a practical OS for running network applications.

### What's Working

- **x86_64 Platform:** Full implementation with preemptive multitasking, ELF loading, interrupt handling, and console output
- **x86_32 Platform:** Complete port supporting 32-bit x86 processors via Multiboot2
- **Multi-platform Architecture:** Clean separation between platform-agnostic kernel and architecture-specific code
- **Preemptive Multitasking:** MLFQ scheduler with interrupt-driven task switching
- **ELF Binary Support:** Loads both 32-bit and 64-bit ELF binaries with relocation support
- **Hardware Abstraction:** Well-defined traits for CPU, schedulers, and memory management

### Known Issues

- **Task Finalization:** Tasks that complete may not be properly removed from the scheduler (see DEVELOPMENT_LOG.md for details)
- **Memory Management:** Basic buddy system allocator in place; more advanced features pending
- **Networking:** TCP/IP stack under development; foundational IPC infrastructure exists

## Goals

- **CLI-First Experience:** RosX is built to be a powerful command-line oriented operating system.
- **Networking as a Priority:** A primary objective is implementing a robust TCP/IP stack to support network applications like web servers.
- **Multi-Platform Support:** While development started on `x86_64`, RosX is designed to be portable across a wide range of architectures, including `x86_32`, ARM, RISC-V, m68k, and various microcontrollers.
- **Microkernel Architecture:** The system follows a modular design, keeping the core kernel lean and moving non-essential services into userspace or pluggable modules.

## Architecture

RosX emphasizes a clean separation between platform-independent logic and architecture-specific implementations.

### Kernel Layer (`kernel/`)
Platform-agnostic code that forms the core OS functionality:
- **Scheduler:** Pluggable scheduler trait with MLFQ and FIFO implementations
- **Task Management:** Task structures, state machine, and context management
- **Memory Management:** Buddy allocator and bitmap-based chunk allocator
- **IPC Infrastructure:** Message-passing system (under development)
- **ELF Loader:** Supports both 32-bit and 64-bit ELF binaries
- **Hardware Abstraction:** Traits for CPU, memory managers, and other subsystems

### Architecture Layer (`arch/`)
Platform-specific implementations that provide hardware support:
- **x86_64:** Full implementation with IDT/PIC setup, interrupt handlers, VGA/framebuffer output
- **x86_32:** Multiboot2-compliant port using 32-bit protected mode
- **Future Platforms:** ARM, RISC-V, m68k designed to follow the same pattern

### Hardware Abstraction Layer (HAL)
The kernel interacts with hardware through well-defined traits:
- **`Cpu` trait:** Provides `setup()`, `enable_interrupts()`, `disable_interrupts()`, and task initialization
- **`Scheduler` trait:** Pluggable scheduling strategies selected at boot time
- **`ElfArch` trait:** Architecture-specific ELF relocation handling

### Pluggable Subsystems
Core components are designed to be interchangeable:
- **Schedulers:** MLFQ (current), FIFO, and future algorithms can be swapped via configuration
- **Memory Managers:** Buddy allocator with pluggable chunk allocators
- **Console Output:** VGA text mode and framebuffer support selectable at compile time

### Memory Management
The system utilizes:
- **Buddy System Allocator:** For physical memory management
- **Bitmap-based Chunk Allocator:** For efficient small allocation
- **No External Dependencies:** Pure `core`/`alloc` implementation for kernel code

## Current State

### x86_64 Platform (Primary)
The current bootstrap platform provides a complete implementation:

- **Preemptive Multitasking:** MLFQ scheduler with both cooperative and interrupt-driven task switching
- **Interrupt Handling:** Full IDT and PIC management for hardware interrupts (keyboard, timer)
- **ELF Loading:** 64-bit ELF binary support with full relocation handling
- **Console Output:** VGA text mode and framebuffer with ANSI escape sequence parsing
- **Context Switching:** Interrupt-driven using `iretq` for uniform task switching
- **System Calls:** Basic infrastructure in place

### x86_32 Platform (Complete)
Full 32-bit x86 support via Multiboot2:

- **Multiboot2 Bootloader Interface:** Standard GRUB-compatible entry point
- **32-bit ELF Support:** Loads both 32-bit and 64-bit ELF binaries
- **PIC and PIT:** 8259A PIC and 8254 PIT configuration
- **System V i386 ABI:** Proper calling convention for task contexts
- **Port I/O:** Direct port access for console output (QEMU debug console)

### Future Platforms
Architectural support planned for:
- **ARM:** 32-bit and 64-bit ARM processors
- **RISC-V:** Modern RISC architecture with user-mode support
- **m68k:** Classic Motorola 68000 series
- **Microcontrollers:** Lightweight variants for embedded systems

## Getting Started

### Prerequisites

To build and run RosX, you will need:

- **Rust Nightly:** `rustup default nightly`
- **Rust Source:** `rustup component add rust-src`
- **QEMU:** For emulation (`qemu-system-x86_64` for x86_64, `qemu-system-i386` for x86_32)

### Build and Run (x86_64)

The x86_64 platform is the current bootstrap target.

1. **Clone the repository:**
   ```bash
   git clone https://github.com/rghisi/rosx.git
   cd rosx
   ```

2. **Build and run:**
   ```bash
   cd arch/x86_64
   cargo run
   ```

   To build only:
   ```bash
   cd arch/x86_64
   cargo build
   ```

### Build and Run (x86_32)

The x86_32 platform runs on older 32-bit processors.

1. **Build:**
   ```bash
   cd arch/x86_32
   cargo build
   ```

2. **Run:**
   ```bash
   cd arch/x86_32
   cargo run
   ```

### Building All Platforms

Use the workspace to build the kernel and in-workspace apps:
```bash
cargo build --workspace
```

## Development

### Project Structure

```
rosx/
├── arch/                    # Architecture-specific implementations
│   ├── x86_64/             # 64-bit x86 implementation
│   ├── x86_32/             # 32-bit x86 implementation (Multiboot2)
│   └── ...                 # Future platforms (ARM, RISC-V, m68k)
├── kernel/                 # Platform-agnostic kernel code
│   ├── src/
│   │   ├── scheduler/      # Pluggable scheduling strategies
│   │   ├── task.rs         # Task structure and management
│   │   ├── memory/         # Memory allocation subsystem
│   │   └── elf/            # ELF binary loader
├── apps/                   # User-space applications
│   ├── shell/              # Command-line shell
│   ├── hello_elf/          # Simple ELF test program
│   └── ...                 # More applications in development
├── collections/            # Custom collections
├── system/                 # System-level utilities
└── usrlib/                 # User-space library
```

### Key Files

- **`kernel/src/kernel.rs`** - Main kernel struct and bootstrap logic
- **`kernel/src/task.rs`** - Task management and context switching
- **`kernel/src/scheduler/`** - Scheduler implementations (MLFQ, FIFO)
- **`arch/x86_64/src/main.rs`** - x86_64 entry point and initialization
- **`arch/x86_32/src/boot.S`** - Multiboot2 bootloader assembly

### Development Guidelines

- **Incremental Development:** Make small, testable changes; verify each step
- **Hardware Abstraction:** All platform-specific code must use traits (HAL pattern)
- **Zero External Dependencies:** Kernel code uses only `core`, `alloc`, and `compiler_builtins`
- **Unit Tests:** Write comprehensive tests for all kernel modules
- **No Comments in Code:** Write self-explanatory code with clear names; exceptions for assembly

See `AGENTS.md` for detailed development guidelines and coding standards.

## Releases

Ready-to-use binary images are available in the [Releases](https://github.com/your-repo/rosx/releases) section of the GitHub repository. You can download these and run them directly in QEMU.

## Documentation

- **`AGENTS.md`** - Detailed development guidelines and coding standards
- **`DEVELOPMENT_LOG.md`** - Session-based development history and recent changes
- **`INTERRUPT_DRIVEN_CONTEXT_SWITCH_REFACTORING.md`** - Technical deep-dive on context switching
- **`X86_32_PORT_PLAN.md`** - 32-bit x86 port implementation details

## Contributing

RosX is a learning project focused on practical operating system development. Contributions are welcome, especially:

- Bug fixes and improvements to existing features
- New platform ports (ARM, RISC-V, m68k)
- Documentation improvements
- Test cases and verification

For major changes, please open an issue first to discuss what you would like to change.

## License

This project is licensed under the MIT License - see the LICENSE file for details.
