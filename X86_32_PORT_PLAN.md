# x86-32 Port Plan

> **Created:** 2026-03-01
> **Branch:** `claude/x86-32bit-port-plan-GblhE`

---

## Portability Findings Summary

### Things That Are Fine (not blocking)

- `usize` for stack_pointer/entry_point/entry_param — `usize` is the native pointer width, adapts correctly
- `stack: [usize; 2048]` — gives 8 KB on 32-bit (16 KB on 64-bit); 8 KB is fine, only the comment is wrong
- `transmute` from `usize` to `fn()` — function pointers are always pointer-sized, portable
- Bitmap allocator — `usize`-based bitmap adapts; `BITS_PER_WORD` becomes 32 on 32-bit
- `MemoryBlock.start/size: usize` — fine for 32-bit, physical addresses fit in 32 bits

### Real Issues to Fix

#### Kernel (`kernel/`) — one real gap

**ELF loader is Elf64-only.** `structs.rs` only has `Elf64*` types; `mod.rs` checks size against
`Elf64Header` and casts directly. On x86-32 we need to load Elf32 binaries (different header/phdr
sizes and field widths).

The `ElfArch::apply_relocation` signature works as-is for 32-bit: the loader reads the implicit
addend from the patch location before calling the trait method, so `X86_32ElfArch` receives
`(base, offset, info, addend)` with the same semantics as the 64-bit path.

#### Architecture layer — everything in `arch/x86_32/` to be created

All x86_64-specific code is correctly isolated in `arch/x86_64/`. The port requires a parallel
`arch/x86_32/` built from scratch.

---

## Implementation Steps

### Step 1 — Kernel: add Elf32 structs [DONE]

Add `Elf32Header`, `Elf32Phdr`, `Elf32Dyn`, `Elf32Rel` to `kernel/src/elf/structs.rs`.

### Step 2 — Kernel: update ELF loader for class detection [DONE]

Update `load_elf` to read `e_ident[4]` (ELF class: `1`=32-bit, `2`=64-bit) and dispatch to a
32-bit or 64-bit load path.

### Step 3 — `arch/x86_32/` scaffolding + target JSON [DONE]

Created directory layout mirroring `arch/x86_64/`. `rosx-i686.json` targets i686-unknown-none:
- `target-pointer-width: 32`, no SSE/MMX (`-mmx,-sse,-sse2`), rust-lld linker, panic=abort
- `+soft-float` rejected by rustc (ABI-incompatible with i686); x87 FPU left as-is
- `pic8259` crate removed — it hard-requires `x86_64` port I/O; PIC will use inline asm

### Step 4 — Multiboot2 entry point [DONE]

- `boot.S`: Multiboot2 header (magic `0xE85250D6`, checksum, end tag), 64 KB `.bss` stack,
  `_start` sets up `esp`, pushes `eax`/`ebx`, calls `kernel_main`
- `linker.ld`: load at 1 MB, `KEEP(*(.multiboot))` prevents lld GC of unreferenced section
- `build.rs`: emits absolute `-T<path>/linker.ld` (lld rejects relative paths)
- `debug_console.rs`: `QemuDebugConsole` via port 0xE9, implements `KernelOutput`
- `main.rs`: `kernel_main` validates Multiboot2 magic, parses memory-map tag (type 6),
  calls `kernel::kernel::bootstrap`

### Step 5 — CPU trait: stack init + context switch [DONE]

- `cpu.rs`: `X86_32` struct implementing `Cpu` trait
- `initialize_stack`: System V i386 ABI (cdecl) — args pushed right-to-left, `esp ≡ 12 mod 16`
  at task entry; callee-saved: ebp, ebx, esi, edi
- `swap_context` in `global_asm!`: `cli` → read args before pushes → null check first →
  conditional save → load new esp → pop registers → `sti` → `ret`
- Decision: `global_asm!` inline (one function, tightly coupled); `boot.S` stays separate

### Step 6 — Interrupts [DONE]

- `interrupts.rs`: hand-rolled 256-entry IDT (no `x86_64` crate), `lazy_static!` init
- IDT entry: `interrupt_gate` type (`0x8E`), code selector `0x08`
- 8259A PIC: full ICW1-ICW4 re-initialisation sequence, master at `0x20`, slave at `0x28`
- PIT: 100 Hz (divisor = 11931), 10 ms/tick
- `AtomicU32` instead of `AtomicU64` — bare-metal i686 has no `target_has_atomic = "64"`
- Rust 2024: explicit `unsafe {}` required inside `unsafe fn` for inline asm

### Step 7 — ELF arch for x86-32 [DONE]

- `elf_arch.rs`: `X86_32ElfArch` implementing `ElfArch`
- `R_386_RELATIVE = 8`: `info & 0xff == 8`, write `(base as i64 + addend) as u32` to patch addr
- `main.rs`: `pub static ELF_ARCH: X86_32ElfArch` exposed for kernel use

### Step 8 — Wire up kernel bootstrap with CPU + ELF arch [DONE]

- `main.rs`: `static KCONFIG: KConfig` bundling `&CPU`, `&ELF_ARCH`, `mfq_scheduler`
- `kernel_main` now calls `Kernel::new(&KCONFIG)` → `kernel.setup()` → `kernel.start()`
- `extern crate alloc` added (required by `Kernel::new` which allocates `Box<dyn Scheduler>`)

---

## Notes

- x86-32 calling convention (cdecl / System V i386 ABI):
  - Args pushed on stack right-to-left, **caller** cleans up
  - Callee-saved: `EBX`, `ESI`, `EDI`, `EBP`
  - Return value in `EAX`
- Bootloader: **Multiboot2** (GRUB) — standard protocol for 32-bit protected-mode kernels
- PIC 8259 and PIT are identical hardware on x86/x86_64 — code can be shared or reused
- No red zone on x86-32 (there's nothing to disable; it's an x86_64-only concept)
- Syscalls: `int 0x80` not SYSCALL instruction (64-bit only)
