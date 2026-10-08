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
- **Hardware Abstraction:** Well-defined traits for CPU and ELF architecture

### Known Issues

- **Memory Management:** Free-list allocator is active; a bitmap-chunk allocator exists but is not yet wired in
- **Networking:** A goal, not yet implemented; mailbox IPC infrastructure is in place

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
- **Memory Management:** Free-list allocator (active) and bitmap-chunk allocator (not yet wired in)
- **IPC Infrastructure:** Mailbox-based message passing (bind/connect/send/receive)
- **ELF Loader:** Supports both 32-bit and 64-bit ELF binaries
- **Hardware Abstraction:** Traits for CPU, ELF architecture, and other subsystems

### Architecture Layer (`arch/`)
Platform-specific implementations that provide hardware support:
- **x86_64:** Full implementation with IDT/PIC setup, interrupt handlers, VGA/framebuffer output
- **x86_32:** Multiboot2-compliant port using 32-bit protected mode
- **Future Platforms:** ARM, RISC-V, m68k designed to follow the same pattern

### Hardware Abstraction Layer (HAL)
The kernel interacts with hardware through well-defined traits:
- **`Cpu` trait:** Provides `setup()`, `enable_interrupts()`, `disable_interrupts()`, and task initialization
- **`ElfArch` trait:** Architecture-specific ELF relocation handling

### Pluggable Subsystems
Core components are designed to be interchangeable:
- **Schedulers:** MLFQ (current), FIFO, and future algorithms can be swapped via configuration
- **Memory:** Free-list global allocator (active); a pluggable allocator is a low-priority goal
- **Console Output:** Multiplexed output (framebuffer + QEMU debug console)

### Memory Management
The system utilizes:
- **Free-list Allocator:** Active global allocator for kernel memory
- **Bitmap-chunk Allocator:** Present, not yet wired in
- **`no_std`:** std is forbidden; dependencies limited to `collections`, `system`, `lazy_static`

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

2. **Build everything (ELF apps + kernel + disk image):**
    ```bash
    cargo xtask build
    ```

3. **Run in QEMU:**
    ```bash
    cargo run -p rosx
    ```

    To build only the kernel:
    ```bash
    cargo build -p rosx --target arch/x86_64/rosx.json
    ```

    `cargo build --workspace` is intentionally unsupported; use `cargo xtask build` or build individual packages.

### Build and Run (x86_32)

The x86_32 platform runs on older 32-bit processors.

1. **Build:**
   ```bash
   cd arch/x86_32
   cargo build -p rosx-x86
   ```

2. **Run:**
   ```bash
   bash arch/x86_32/build-image.sh
   ```

### Testing

```bash
cargo xtask test
```

### More detail

The commands above are enough to get a boot. For the full flow — the custom runner and disk-image creation, the standalone user-space ELF apps, and the x86_32 bootable ISO — see **[AGENTS.md](AGENTS.md) → *Build & Run***.

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
├── apps/                   # User-space applications (workspace members)
│   ├── shell/              # Command-line shell
│   ├── hello_elf/          # Simple ELF test program
│   ├── random_gen_server/  # Random number generator server
│   ├── snake/              # Snake game
│   ├── tetris/             # Tetris game
│   ├── conway/             # Conway's Game of Life
│   ├── dummy/              # Dummy app for testing
│   └── test_suite/         # Test suite app
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
- **`no_std` kernel:** std is forbidden; dependencies are limited to `collections`, `system`, and `lazy_static`
- **Unit Tests:** Write comprehensive tests for all kernel modules
- **No Comments in Code:** Write self-explanatory code with clear names; exceptions for assembly

See `AGENTS.md` for detailed development guidelines and coding standards.

## Releases

Ready-to-use binary images are available in the [Releases](https://github.com/rghisi/rosx/releases) section of the GitHub repository. You can download these and run them directly in QEMU.

## Documentation

- **`AGENTS.md`** - Detailed development guidelines, coding standards, and the full build/test/run flow
- **`plans/`** - In-flight design notes (e.g. `notification-based-futures.md`)

## Contributing

RosX is a learning project focused on practical operating system development. Contributions are welcome, especially:

- Bug fixes and improvements to existing features
- New platform ports (ARM, RISC-V, m68k)
- Documentation improvements
- Test cases and verification

For major changes, please open an issue first to discuss what you would like to change.

## License

This project is licensed under the MIT License.
