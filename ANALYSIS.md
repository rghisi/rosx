# Comparative Analysis: RosX vs. MS-DOS 6.22

## Introduction
This report provides a deep architectural comparison between **RosX**, a modern hobby operating system developed in Rust, and **MS-DOS 6.22**, the pinnacle of the classic DOS era (circa 1994). While both aim for a CLI-first experience, they represent two radically different eras of computing philosophy: the "wild west" of 16-bit real-mode hardware access versus the safety and abstraction of 64-bit modern systems programming.

---

## 1. Architectural Core: CPU Mode and Memory

| Feature | MS-DOS 6.22 | RosX (Current) |
|:---|:---|:---|
| **Processor Mode** | 16-bit Real Mode | 64-bit Long Mode |
| **Address Space** | 1 MB (Segmented) | 16 EB (Flat) |
| **Memory Model** | Segment:Offset (CS, DS, SS, ES) | Flat (Zero-based or offset) |
| **Memory Management**| Manual (HIMEM.SYS, EMM386.EXE) | Buddy Allocator (Automatic) |
| **Protection** | None (Any app can crash kernel) | Kernel/User separation (Planned/Basic) |

### MS-DOS 6.22: The Segmented Struggle
MS-DOS was built for the Intel 8088. Its 1 MB address space was divided into "Conventional Memory" (640 KB) and the "Upper Memory Area" (384 KB). To go beyond 1 MB, it used complex hacks:
- **XMS (Extended Memory):** Using HIMEM.SYS to access memory above 1 MB.
- **EMS (Expanded Memory):** Paging 16 KB banks through a 64 KB "window" in the UMA.
- **Segments:** Programmers had to deal with 64 KB segment limits constantly.

### RosX: The Modern Flat Model
RosX bypasses all these historical baggage. It boots directly into **Long Mode**, providing a massive, flat 64-bit address space.
- **Buddy Allocator:** RosX uses a sophisticated `LockedHeap` buddy allocator for physical memory management, which is significantly more advanced than the simple "First Fit" MCB (Memory Control Block) chain used by DOS.
- **Safety:** By using Rust and modern CPU features, RosX is designed to prevent the "spaghetti memory" issues where one DOS application could accidentally overwrite the BIOS or the OS kernel itself.

---

## 2. Tasking & Scheduling: Parallelism vs. Persistence

| Feature | MS-DOS 6.22 | RosX (Current) |
|:---|:---|:---|
| **Multitasking** | Single-tasking (mostly) | Preemptive Multitasking |
| **Scheduler** | None (Run to completion) | MLFQ (Multi-level Feedback Queue) |
| **Background Tasks** | TSRs (Terminate and Stay Resident) | Native Kernel Threads |
| **Context Switching** | Manual / Interrupt hooking | Automatic (Timer/Keyboard/Yield) |

### MS-DOS 6.22: One Thing at a Time
DOS was never designed as a multitasking OS. When you ran a program, it took total control of the CPU.
- **TSRs:** Programs like *Sidekick* or mouse drivers would "stay resident" by hooking hardware interrupts (like the timer or keyboard). This was fragile; if TSRs were loaded in the wrong order, the system would lock up.

### RosX: The Preemptive Powerhouse
RosX features a **Multi-level Feedback Queue (MLFQ)** scheduler.
- **Preemption:** RosX can forcibly switch tasks if one runs too long (Quantum exhaustion), ensuring the system remains responsive.
- **Dynamic Priority:** The MLFQ algorithm automatically demotes CPU-heavy tasks and promotes interactive tasks (like the shell), a concept that didn't exist in standard DOS.

---

## 3. System API & Executables

| Feature | MS-DOS 6.22 | RosX (Current) |
|:---|:---|:---|
| **API Entry Point** | Software Interrupt `INT 21h` | Syscall Dispatcher |
| **Binary Format** | .COM (Flat) / .EXE (MZ) | .ELF (Executable and Linkable) |
| **Relocation** | Minimal (MZ Header) | Dynamic Relocations (Relative) |
| **Dynamic Linking** | No (Static only) | Yes (Supported in ELF loader) |

### MS-DOS 6.22: The Interrupt Table
DOS used the `INT` instruction to call services. AH would hold the function number (e.g., `AH=09h` for Print String).
- **Format:** `.COM` files were simple memory images (limited to 64 KB). `.EXE` files (MZ) allowed for multiple segments and simple relocation.

### RosX: The ELF Standard
RosX uses the industry-standard **ELF** format.
- **Sophistication:** The RosX ELF loader (`kernel/src/elf.rs`) supports parsing program headers and applying **relative relocations**. This allows RosX to support modern features like Address Space Layout Randomization (ASLR) in the future, which was unthinkable in the DOS era.

---

## 4. I/O and File System

| Feature | MS-DOS 6.22 | RosX (Current) |
|:---|:---|:---|
| **Primary FS** | FAT16 | Minimal `File` Trait (Planned) |
| **File Names** | 8.3 (e.g. `COMMAND.COM`) | Long filenames supported |
| **HAL** | IO.SYS / BIOS Interrupts | Rust Traits (`Cpu`, `KernelOutput`) |
| **Hardware Access** | Direct I/O (IN/OUT) | Abstracted through Kernel |

### MS-DOS 6.22: FAT16 and BIOS
DOS was inextricably tied to the **FAT** file system. It relied heavily on the **BIOS** for disk I/O (`INT 13h`) and video (`INT 10h`).

### RosX: Abstracted Architecture
RosX uses a **Hardware Abstraction Layer (HAL)** approach via Rust traits.
- **Portability:** Unlike DOS, which was "hardcoded" for IBM PC clones, RosX's use of traits like `Cpu` allows the kernel to be ported to ARM or RISC-V with minimal changes. Current I/O is handled via modern abstractions like `MultiplexOutput` for VGA and Serial consoles.

---

## 5. Networking: The 90s Frontier

| Feature | MS-DOS 6.22 | RosX (Current/Planned) |
|:---|:---|:---|
| **Standard** | None (Add-on stacks only) | Built-in Networking (Planned) |
| **Architecture** | NDIS/Packet Driver (TSR-based) | Modern Async/Future-based |
| **API** | NetBIOS, Socket (via WINSOCK.DLL) | Rust `async/await` / Futures |

### MS-DOS 6.22: The Networking Afterthought
To get DOS online in the 90s, you usually used the **Microsoft Network Client 3.0**. It was a collection of TSRs that ate up precious conventional memory. It was notoriously difficult to configure (IRQ/DMA settings) and lacked a native TCP/IP stack until late in its life.

### RosX: Born for the Web
RosX is designed with networking as a core goal.
- **Async I/O:** RosX already uses a **Future-based system** (`TimeFuture`, `KeyboardFuture`). This makes implementing a high-performance TCP/IP stack much more natural than the synchronous, interrupt-heavy model of DOS.

---

## 6. Roadmap: Achieving "80s/90s Standards"
To reach the functionality of a de-facto 90s workstation (like a DOS 6.22 + Windows 3.1 or a Slackware 1.0 system), RosX needs:

1.  **Hierarchical File System (FAT32/ext2):** Current RosX is missing a persistent disk-based file system. Implementing FAT32 would provide the best "90s feel" and compatibility.
2.  **Shell Sophistication:** A shell that supports piping (`|`), redirection (`>`), and batch scripting (`.BAT`).
3.  **Standard C Library (libc):** To run classic C code, a robust syscall-to-libc mapping is essential.
4.  **Network Stack (IP/TCP):** A "90s standard" requires at least `ping`, `ftp`, and `telnet` capability.

---

## 7. "What If" - The Planned Features
*   **What if RosX goes Multi-Platform?**
    Unlike DOS, which died with the x86 architecture, RosX could run on a modern ARM laptop or a RISC-V microcontroller. This makes it more like **UNIX** than DOS.
*   **What if RosX implements VFS (Virtual File System)?**
    RosX could mount multiple "drives" seamlessly (Floppy, Hard Drive, Network Share), something DOS struggled with (requiring `LASTDRIVE` and `MSCDEX`).
*   **What if RosX adds User Space Protection?**
    This would make RosX a "Real OS" compared to DOS. A crashing app would just be terminated by the kernel, whereas in DOS, it usually meant reaching for the Reset button.

---

## Final Summary
RosX is a **"Post-Modern DOS."** It captures the simplicity and CLI-focus of the 90s but builds it on a foundation of 21st-century safety (Rust) and architectural power (64-bit, MLFQ). While DOS was a collection of clever hacks to overcome hardware limits, RosX is a clean-slate design that treats the hardware with respect while providing modern abstractions.
