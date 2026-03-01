# RosX

RosX is a hobbyist operating system written in Rust from scratch. It is designed with a focus on portability, a microkernel-inspired architecture, and networking capabilities. A key design goal is to not require an MMU, enabling it to run on smaller and simpler devices.

## Goals

- **CLI-First Experience:** RosX is built to be a powerful command-line oriented operating system.
- **Networking as a Priority:** A primary objective is implementing a robust TCP/IP stack to support network applications like web servers.
- **Multi-Platform Support:** While development started on `x86_64`, RosX is designed to be portable across a wide range of architectures, including `x86_32`, ARM, RISC-V, m68k, and various microcontrollers.
- **Microkernel Architecture:** The system follows a modular design, keeping the core kernel lean and moving non-essential services into userspace or pluggable modules.

## Architecture

RosX emphasizes a clean separation between platform-independent logic and architecture-specific implementations.

- **Hardware Abstraction Layer (HAL):** The kernel interacts with hardware through well-defined traits (e.g., the `Cpu` trait), ensuring the core logic remains portable.
- **Pluggable Subsystems:** Core components like schedulers and memory managers are designed to be pluggable. They are selected and configured during the kernel bootstrapping phase.
- **Memory Management:** The system utilizes a Buddy system allocator for physical memory management and a bitmap-based chunk allocator.
- **Inter-Process Communication (IPC):** A message-passing IPC system is currently under development to facilitate communication between isolated tasks.

## Current State

The current implementation focuses on the `x86_64` architecture and includes:

- **Preemptive Multitasking:** A Multi-level Feedback Queue (MLFQ) scheduler supports both cooperative and interrupt-driven task switching.
- **ELF Binary Support:** Loads 64-bit ELF binaries with full relocation support.
- **Interrupt Handling:** Robust IDT and PIC management for handling hardware interrupts (keyboard, timer, etc.).
- **Console Output:** Support for both VGA text mode and modern framebuffers with ANSI escape sequence parsing.
- **Foundational IPC:** Basic infrastructure for synchronous and asynchronous message passing (Work in Progress).

## Getting Started

### Prerequisites

To build and run RosX, you will need:

- **Rust Nightly:** `rustup default nightly`
- **Rust Source:** `rustup component add rust-src`
- **LLVM Tools:** `rustup component add llvm-tools-preview`
- **QEMU:** For emulation (`qemu-system-x86_64`)

### Build and Run (x86_64)

The current bootstrap platform is `x86_64`.

1. **Clone the repository:**
   ```bash
   git clone https://github.com/your-repo/rosx.git
   cd rosx
   ```

2. **Run the OS:**
   You can use the built-in runner to build and launch the OS in QEMU:
   ```bash
   cd arch/x86_64
   cargo run
   ```

   Alternatively, to build only:
   ```bash
   cd arch/x86_64
   cargo build
   ```

## Releases

Ready-to-use binary images are available in the [Releases](https://github.com/your-repo/rosx/releases) section of the GitHub repository. You can download these and run them directly in QEMU.
