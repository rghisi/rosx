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

Secondary consequence: `ElfArch::apply_relocation(info: u64, addend: i64)` is designed for Elf64
RELA. Elf32 uses REL (no addend — the patched word encodes the base). Trait needs to accommodate
this.

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

### Step 3 — Kernel: update `ElfArch` trait for REL support [ ]

Add `apply_rel_relocation(&self, base: usize, offset: usize, info: u32)` with a default no-op,
so existing `X86_64ElfArch` doesn't break.

### Step 4 — `arch/x86_32/` scaffolding + target JSON [ ]

Create directory layout mirroring `arch/x86_64/`. Write `rosx-i686.json`
(`target-pointer-width: 32`, `disable-redzone: true`, no SSE).

### Step 5 — Multiboot2 entry point [ ]

Embed a Multiboot2 header, implement `kernel_main` that receives the Multiboot2 info pointer,
extracts memory regions into `MemoryBlocks`, bootstraps the kernel.

### Step 6 — CPU trait: stack init + context switch [ ]

`cpu.rs` (`X86_32`) + `context_switching.S` for x86-32 cdecl ABI.
Only 4 callee-saved registers (EBX, ESI, EDI, EBP) vs 15 on x86_64.

### Step 7 — Interrupts [ ]

`interrupts.rs`: 32-bit IDT, PIC 8259 (same hardware as x86_64!), PIT timer (same!).
Drop MSR/SYSCALL setup (x86_64-only); use `int 0x80` for syscalls instead.

### Step 8 — ELF arch for x86-32 [ ]

`elf_arch.rs`: `X86_32ElfArch` implementing `ElfArch`.
`R_386_RELATIVE = 8` in Elf32Rel: `*patch_addr = (*patch_addr).wrapping_add(base as u32)`.

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
