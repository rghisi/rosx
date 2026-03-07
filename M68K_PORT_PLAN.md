# m68k Port Plan for RosX

## Context

RosX already runs on x86_64 and x86_32 under QEMU. The project's trait-based design (`Cpu`, `ElfArch`) was built for exactly this kind of multi-platform expansion. This plan describes how to add a `arch/m68k/` port targeting the Motorola 68000 (classic) CPU, tested under QEMU, following the same pattern as the x86_32 port.

**Target:** 68000 classic CPU, pure QEMU emulation (no real hardware yet).

---

## Critical Pre-Condition: 68000 VBR Constraint

The 68000 has **no VBR register** — the exception vector table is **hardwired at physical address 0x00000000**. QEMU must map RAM starting at 0x00000000 for this to work. If not, fall back to `-cpu m68010` (nearly identical ISA, but adds VBR to relocate the table into RAM wherever it lives).

This must be verified in Phase 0 before anything else.

---

## Phase Status

| Phase | Description | Status |
|-------|-------------|--------|
| 0 | Toolchain & QEMU verification | ✅ Done |
| 1 | Crate skeleton & target JSON | ⬜ Pending |
| 2 | Linker script & boot code | ⬜ Pending |
| 3 | CPU trait (no context switch yet) | ⬜ Pending |
| 4 | Context switching (`swap_context` assembly) | ⬜ Pending |
| 5 | Exception handling & timer | ⬜ Pending |
| 6 | Serial console output | ⬜ Pending |
| 7 | Kernel bootstrap wire-up | ⬜ Pending |
| 8 | ELF architecture support | ⬜ Pending |
| 9 | End-to-end testing & bug fixing | ⬜ Pending |

---

## Phase 0 — Toolchain & QEMU Verification

**Goal:** Confirm the toolchain compiles m68k bare-metal Rust and QEMU can boot it.
**Status:** ✅ Done

### 0.1 Rust/LLVM compilation — Results

**Working toolchain:** `nightly-2024-04-01` (LLVM 18.1.2)

Newer nightlies (LLVM 19+) crash with SIGSEGV in `SelectionDAGISel::CodeGenAndEmitDAG` because the m68k backend has no instruction selection patterns for `AtomicLoad`/`AtomicStore` (used by `compiler_builtins::mem::memcpy_element_unordered_atomic`). Fixed by `"max-atomic-width": 0` in target JSON.

`rust-lld` (bundled) crashes with SIGILL on m68k ELF; system `ld.lld` (LLVM 18) crashes with SIGSEGV. **Must use `m68k-linux-gnu-ld` (GNU binutils).**

**Final working target JSON (`rosx-m68k.json`):**
```json
{
    "llvm-target": "m68k-unknown-none",
    "data-layout": "E-m:e-p:32:16:32-i8:8:8-i16:16:16-i32:16:32-n8:16:32-a:0:16-S16",
    "arch": "m68k",
    "target-endian": "big",
    "target-pointer-width": "32",
    "target-c-int-width": "32",
    "os": "none",
    "executables": true,
    "linker-flavor": "gnu",
    "linker": "m68k-linux-gnu-ld",
    "panic-strategy": "abort",
    "cpu": "M68000",
    "features": "-isa-68881,-isa-68882",
    "max-atomic-width": 0
}
```

**`.cargo/config.toml`:**
```toml
[build]
target = "path/to/rosx-m68k.json"

[unstable]
build-std = ["core", "compiler_builtins"]
build-std-features = []
```

**Required nightly feature flags in `main.rs`:**
```rust
#![feature(asm_experimental_arch)]
#![feature(asm_const)]
```

**LLVM m68k inline asm limitations (important for Phase 3):**
- `#` is the comment character in LLVM's m68k inline asm parser — `ori.w #0x0700, %sr` does NOT work
- Pre/post-decrement addressing `-(sp)` / `(sp)+` not supported
- `sr` / `%sr` not recognised as the Status Register
- **Workaround A:** Raw `.short` bytes in `asm!` for SR and MOVEM:
  ```rust
  asm!(".short 0x007C, 0x0700", options(nomem, nostack)); // ORI.W #0x0700, SR
  asm!(".short 0x027C, 0xF8FF", options(nomem, nostack)); // ANDI.W #0xF8FF, SR
  asm!(".short 0x40C0", out("d0") sr, options(nomem, nostack)); // MOVE.W SR, D0
  asm!(".short 0x48E7, 0x3F3E", options(nostack));  // MOVEM.L D2-D7/A2-A6, -(SP)
  asm!(".short 0x4CDF, 0x7CFC", options(nostack));  // MOVEM.L (SP)+, D2-D7/A2-A6
  ```
- **Workaround B (preferred):** Put boot/context-switch assembly in `.S` files compiled by `m68k-linux-gnu-as` (full AT&T syntax support). This is how x86_32 handles its assembly too.

**AtomicU32 / kernel atomics:** With `max-atomic-width: 0`, Rust atomics on m68k will use interrupt-disable sequences (correct for single-core). The kernel's `AtomicU32` uses in `interrupts.rs` (`SYSTEM_TIME_MS`) will still compile but require interrupt masking — acceptable.

### 0.2 QEMU machine discovery — Results

**Machine:** `qemu-system-m68k -machine virt -cpu m68000`
**Confirmed:** RAM starts at **0x00000000** — 68000 without VBR works. ✅

**Verified by booting a minimal kernel that printed `ROSX` to serial.**

**QEMU virt machine memory map:**

| Device | Base Address | Size | CPU IRQ |
|--------|-------------|------|---------|
| RAM | 0x00000000 | up to 32 MB | — |
| Goldfish PIC #0 | 0xff000000 | 0x1000 | → CPU IRQ #1 |
| Goldfish PIC #1 | 0xff001000 | 0x1000 | → CPU IRQ #2 |
| Goldfish PIC #2 | 0xff002000 | 0x1000 | → CPU IRQ #3 |
| Goldfish PIC #3 | 0xff003000 | 0x1000 | → CPU IRQ #4 |
| Goldfish PIC #4 | 0xff004000 | 0x1000 | → CPU IRQ #5 |
| Goldfish PIC #5 | 0xff005000 | 0x1000 | → CPU IRQ #6 |
| Goldfish RTC #1 (timer) | 0xff006000 | 0x1000 | PIC #5 IRQ #1 → CPU IRQ #6 |
| Goldfish RTC #2 | 0xff007000 | 0x1000 | PIC #5 IRQ #2 → CPU IRQ #6 |
| Goldfish TTY (UART) | 0xff008000 | 0x1000 | PIC #0 IRQ #32 → CPU IRQ #1 |
| Virt controller | 0xff009000 | 0x1000 | PIC #0 IRQ #1 → CPU IRQ #1 |

**UART (Goldfish TTY at 0xff008000):**
- Write a byte: write 32-bit value to offset 0x00 (REG_PUT_CHAR) — verified working

**Timer (Goldfish RTC at 0xff006000):**
- No periodic mode. Must re-arm after each interrupt:
  1. Enable IRQ: write 1 to offset 0x10 (RTC_IRQ_ENABLED)
  2. Set alarm: write next expiry time (nanoseconds) to 0x08/0x0c (RTC_ALARM_LOW/HIGH) then 0x08 triggers arming
  3. On interrupt: read `0x00` (RTC_TIME_LOW) to latch current time, clear interrupt by writing to 0x1c (RTC_CLEAR_INTERRUPT), re-arm

**PIC (Goldfish PIC at 0xff000000 + n*0x1000):**
- Enable interrupt source: write bitmask to offset 0x10 (REG_ENABLE)
- Read pending: read offset 0x04 (REG_IRQ_PENDING)
- Acknowledge: write bitmask to offset 0x0c (REG_DISABLE), then re-enable via REG_ENABLE

**QEMU run command (confirmed working):**
```bash
qemu-system-m68k \
  -machine virt \
  -cpu m68000 \
  -m 32M \
  -kernel target/rosx-m68k/debug/rosx-m68k \
  -nographic \
  -monitor none \
  -serial stdio \
  -no-reboot
```

---

## Phase 1 — Crate Skeleton & Target Setup

**Status:** ⬜ Pending

### Directory structure (mirrors `arch/x86_32/`)

```
arch/m68k/
├── Cargo.toml
├── rosx-m68k.json
├── rust-toolchain.toml
├── linker.ld
├── .cargo/
│   └── config.toml
└── src/
    ├── main.rs
    ├── cpu.rs
    ├── interrupts.rs
    ├── boot.S
    ├── serial.rs          # replaces vga_buffer + debug_console
    └── elf_arch.rs
```

### `rosx-m68k.json`

Key differences from `rosx-i686.json`:
- `"llvm-target": "m68k-unknown-none"`
- `"target-endian": "big"` ← first big-endian target in the project
- `"data-layout": "E-m:e-p:32:32-i8:8-i16:16-i32:32-n32"` (`E` = big-endian)
- `"features": "+m68000"`

```json
{
    "llvm-target": "m68k-unknown-none",
    "data-layout": "E-m:e-p:32:32-i8:8-i16:16-i32:32-n32",
    "arch": "m68k",
    "target-endian": "big",
    "target-pointer-width": "32",
    "target-c-int-width": "32",
    "os": "none",
    "executables": true,
    "linker-flavor": "ld.lld",
    "linker": "rust-lld",
    "panic-strategy": "abort",
    "features": "+m68000"
}
```

### `.cargo/config.toml`

```toml
[build]
target = "../rosx-m68k.json"

[unstable]
build-std = ["core", "alloc", "compiler_builtins"]
build-std-features = ["compiler-builtins-mem"]
```

---

## Phase 2 — Linker Script & Boot Code

**Status:** ⬜ Pending

### `linker.ld` (assuming RAM at 0x000000)

```ld
ENTRY(_start)

SECTIONS {
    . = 0x00000000;
    .vectors ALIGN(4) : { KEEP(*(.vectors)) }
    .text    ALIGN(4) : { *(.text .text.*)   }
    .rodata  ALIGN(4) : { *(.rodata .rodata.*) }
    .data    ALIGN(4) : { *(.data .data.*)   }
    .bss ALIGN(4) : {
        _bss_start = .;
        *(COMMON)
        *(.bss .bss.*)
        _bss_end = .;
    }
    kernel_end = .;
}
```

If RAM is elsewhere (68010 fallback), adjust `. = 0x40000000;` and add VBR setup in boot.S.

### `boot.S`

On 68000, the CPU reads `vector[0]` as the initial SP and `vector[1]` as the reset PC before any code runs. The entire 1 KB vector table (256 × 4-byte entries) must be the first thing in the binary.

```asm
.section .vectors, "a"
.global _vectors
_vectors:
    .long  boot_stack_top        /* vector 0: initial SP                  */
    .long  _start                /* vector 1: reset PC (first instruction) */
    .rept  254
    .long  default_exception     /* vectors 2-255: default handler         */
    .endr

.section .bss
.align 4
boot_stack_bottom:
    .skip 65536
boot_stack_top:

.section .text
.global _start
_start:
    lea.l  boot_stack_top, %a7  /* reinitialise SP (redundant but explicit) */
    /* zero BSS */
    lea.l  _bss_start, %a0
    lea.l  _bss_end,   %a1
.zero_bss:
    cmpa.l %a1, %a0
    bge.s  .bss_done
    clr.l  (%a0)+
    bra.s  .zero_bss
.bss_done:
    jsr    kernel_main

.hang:
    stop   #0x2700
    bra.s  .hang

default_exception:
    stop   #0x2700
    bra.s  default_exception
```

---

## Phase 3 — CPU Trait Implementation

**Status:** ⬜ Pending

**Critical file:** `kernel/src/cpu.rs` — defines the `Cpu` trait to implement.

### Key methods

**Interrupt control** — m68k uses IPL bits [10:8] of the Status Register (SR):

```rust
fn enable_interrupts(&self) {
    // IPL = 0 → accept all interrupts
    unsafe { asm!("andi.w #0xF8FF, %sr", options(nomem, nostack)) };
}
fn disable_interrupts(&self) {
    // IPL = 7 → mask all maskable interrupts
    unsafe { asm!("ori.w #0x0700, %sr", options(nomem, nostack)) };
}
fn are_interrupts_enabled(&self) -> bool {
    let sr: u16;
    unsafe { asm!("move.w %sr, {0}", out(reg) sr, options(nomem, nostack)) };
    (sr >> 8) & 0x7 == 0
}
fn halt(&self) {
    unsafe { asm!("stop #0x2700", options(nomem, nostack)) };
}
```

**`initialize_stack`** — builds the fake stack frame that `swap_context` will restore.
Callee-saved registers: D2–D7 + A2–A6 = **11 registers × 4 bytes = 44 bytes**.

Stack layout (top = lowest address, grows down):
```
[sp +  0..40]: D2-D7, A2-A6 (11 regs, via MOVEM)
[sp + 44]:     entry_point   ← RTS jumps here
[sp + 48]:     0             (sentinel — crash if task returns)
[sp + 52]:     param1
[sp + 56]:     param2
```

---

## Phase 4 — Context Switching

**Status:** ⬜ Pending

Uses `MOVEM` — a single m68k instruction that saves/restores a list of registers atomically.
This replaces the x86_32 sequence of individual `push`/`pop` instructions.

### `swap_context` (global_asm! or .S file)

```asm
/* swap_context(store: *mut usize, load: usize)
 * m68k cdecl: [sp+4]=store, [sp+8]=load
 */
.global swap_context
swap_context:
    ori.w  #0x0700, %sr               /* disable interrupts                */
    move.l 4(%sp), %a0                /* a0 = store ptr                    */
    move.l 8(%sp), %a1                /* a1 = load value                   */
    movem.l %d2-%d7/%a2-%a6, -(%sp)  /* push 11 callee-saved regs         */
    tst.l  %a0
    beq.s  1f
    move.l %sp, (%a0)                /* *store = sp                        */
1:
    move.l %a1, %sp                  /* switch to next task's stack        */
    movem.l (%sp)+, %d2-%d7/%a2-%a6 /* pop 11 callee-saved regs           */
    andi.w #0xF8FF, %sr              /* re-enable interrupts               */
    rts                              /* jump to task (pops entry_point)    */
```

`MOVEM.L Dx-Dx/Ax-Ax, -(SP)` saves in register-number order (lowest first) to decreasing addresses. `MOVEM.L (SP)+, ...` restores in the symmetric order.

---

## Phase 5 — Exception Handling & Timer

**Status:** ⬜ Pending

### Interrupt architecture (m68k vs x86)

| x86_32 | m68k |
|--------|------|
| IDT (256 entries, LIDT instruction) | Vector table (256 × 4-byte, at 0x000000) |
| 8259A PIC (port I/O) | MMIO interrupt controller (machine-specific) |
| 8254 PIT timer (port I/O) | MMIO timer (GOLDFISH or similar, machine-specific) |
| `cli`/`sti` | `ori.w #0x0700,%sr` / `andi.w #0xF8FF,%sr` |
| `int 0x80` syscall | `TRAP #0` (vector 32) |

### Steps
1. Fill in specific exception vector entries in `boot.S` (or a Rust init function that writes to vector table in RAM).
2. Discover timer MMIO from QEMU machine (Phase 0 output).
3. Implement timer ISR: increment `SYSTEM_TIME_MS`, ACK the interrupt, call `kernel().preempt()`.
4. Register the ISR address in the auto-vector slot (vector 25 = IRQ level 1).
5. Implement interrupt ACK via MMIO interrupt controller.

---

## Phase 6 — Serial Console Output

**Status:** ⬜ Pending

No VGA on m68k. No port I/O. Replace `vga_buffer.rs` + `debug_console.rs` with a single `serial.rs` using MMIO UART.

```rust
pub struct M68KSerial { base: *mut u8 }

impl M68KSerial {
    pub const fn new(base: usize) -> Self {
        M68KSerial { base: base as *mut u8 }
    }
    fn write_byte(&self, b: u8) {
        // Poll TX-ready bit, then write to TX register
        unsafe { core::ptr::write_volatile(self.base, b) };
    }
}
```

Implement `kernel::default_output::KernelOutput` (same pattern as x86_32).

UART MMIO base address confirmed in Phase 0.2. QEMU invocation: `-serial stdio`.

---

## Phase 7 — Kernel Bootstrap Wire-up

**Status:** ⬜ Pending

Without Multiboot there is no memory map. **Hardcode** for QEMU (`-m 32M` → 32 MB RAM):

```rust
#[unsafe(no_mangle)]
extern "C" fn kernel_main() -> ! {
    let kernel_end = unsafe { &raw const kernel_end_sym as usize };
    let memory_blocks = MemoryBlocks { /* kernel_end..32MB */ };

    kernel::kernel::bootstrap(&memory_blocks, &SERIAL_OUTPUT);
    let mut k = Kernel::new(&KCONFIG);
    k.setup();
    k.schedule(FunctionTask::new("Shell", shell::shell::main));
    k.start();
    panic!("kernel.start() returned");
}
```

Follow `arch/x86_32/src/main.rs` exactly — only the memory-discovery and output-device parts differ.

---

## Phase 8 — ELF Architecture Support

**Status:** ⬜ Pending

m68k ELF: EM_68K = 4, big-endian (ELFDATA2MSB). Key relocation:

```rust
const R_68K_RELATIVE: u64 = 22;  // compare: R_386_RELATIVE = 8 in x86_32

pub struct M68KElfArch;

impl ElfArch for M68KElfArch {
    fn apply_relocation(&self, base: usize, offset: usize, info: u64, addend: i64) {
        if info & 0xff == R_68K_RELATIVE {
            let patch_addr = (base + offset) as *mut u32;
            let value = (base as i64 + addend) as u32;
            // On a big-endian CPU, write_volatile stores in big-endian order natively
            unsafe { core::ptr::write(patch_addr, value) };
        }
    }
}
```

Reference: `arch/x86_32/src/elf_arch.rs` for the pattern.

---

## Phase 9 — Testing

**Status:** ⬜ Pending

### Build

```bash
cd arch/m68k
cargo build
```

### Run

```bash
qemu-system-m68k \
  -machine virt \
  -cpu m68000 \
  -m 32M \
  -kernel target/rosx-m68k/debug/rosx-m68k \
  -nographic \
  -serial stdio
```

### Debug

```bash
# Terminal 1
qemu-system-m68k ... -s -S

# Terminal 2 — standard GDB has m68k support built in
gdb target/rosx-m68k/debug/rosx-m68k \
  -ex "target remote :1234" \
  -ex "break kernel_main" \
  -ex "continue"
```

### Milestones (in order)

1. Binary loads and enters `kernel_main` without crashing (QEMU shows something)
2. Serial output visible in terminal
3. Kernel boots to the scheduler
4. Multiple tasks run and yield (round-robin observed in output)
5. Timer interrupt fires and `kernel().preempt()` works
6. Shell task responsive

---

## Challenges & Risks

| Challenge | Severity | Mitigation |
|-----------|----------|------------|
| 68000 no VBR — vector table fixed at 0x000000 | **High** | Verify QEMU RAM map in Phase 0; fall back to 68010 |
| LLVM m68k backend for bare-metal Rust | **Medium** | Phase 0 compile test; may need nightly pin |
| 68000 has no CAS → `AtomicU32` may not codegen | **Medium** | Replace with interrupt-disable critical sections |
| QEMU machine-specific timer/interrupt MMIO addresses | **Medium** | Identify from QEMU source / `info mtree` in Phase 0 |
| Big-endian (first BE target) | **Low–Medium** | Rust types are endian-neutral; audit ELF + hardware register code |
| No Multiboot — no memory map from bootloader | **Low** | Hardcode for QEMU; extensible to device tree later |
| No VGA, no port I/O | **Low** | MMIO UART via `-serial stdio` |

---

## File Mapping: x86_32 → m68k

| x86_32 file | m68k file | Primary difference |
|-------------|-----------|-------------------|
| `arch/x86_32/src/boot.S` | `arch/m68k/src/boot.S` | Exception vector table instead of Multiboot header |
| `arch/x86_32/src/cpu.rs` | `arch/m68k/src/cpu.rs` | MOVEM context switch, SR interrupt control |
| `arch/x86_32/src/interrupts.rs` | `arch/m68k/src/interrupts.rs` | MMIO controller instead of PIC/PIT port I/O |
| `arch/x86_32/src/elf_arch.rs` | `arch/m68k/src/elf_arch.rs` | `R_68K_RELATIVE` (22), big-endian |
| `arch/x86_32/src/vga_buffer.rs` + `debug_console.rs` | `arch/m68k/src/serial.rs` | MMIO UART only |
| `arch/x86_32/src/main.rs` | `arch/m68k/src/main.rs` | Hardcoded memory map, no Multiboot magic |
| `arch/x86_32/rosx-i686.json` | `arch/m68k/rosx-m68k.json` | Big-endian, m68k LLVM target |
| `arch/x86_32/linker.ld` | `arch/m68k/linker.ld` | `.vectors` section at 0x000000 |
| `arch/x86_32/src/ansi_parser.rs` | copy or shared | No changes needed |

**Kernel files that stay unchanged:** everything under `kernel/src/` — the platform-agnostic core is the whole point.
