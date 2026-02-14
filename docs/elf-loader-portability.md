# ELF Loader Portability Plan

## Current State

The ELF loader (`kernel/src/elf.rs`) is hardcoded for x86_64:
- `R_X86_64_RELATIVE` relocation type (value 8)
- ELF64 structs only
- Writes `u64` when patching relocations
- No `e_machine` validation

## What's Arch-Specific

1. **Relocation type constants** — x86_64: `R_X86_64_RELATIVE` (8), ARM64: `R_AARCH64_RELATIVE` (1027), RISC-V: `R_RISCV_RELATIVE` (3)
2. **Pointer size in relocation patching** — `u64` on 64-bit, `u32` on 32-bit
3. **ELF class** — ELF64 vs ELF32 (different struct layouts)
4. **`e_machine` validation** — should verify the ELF matches the running architecture

## Proposed Abstraction

### ElfRelocator trait (runtime, via KConfig)

```rust
pub trait ElfRelocator {
    fn apply_relocation(&self, base: usize, offset: usize, info: u64, addend: i64);
}
```

Each arch implements which relocation types it supports. Injected via KConfig:

```rust
pub struct KConfig {
    pub cpu: &'static dyn Cpu,
    pub elf_relocator: &'static dyn ElfRelocator,
    pub idle_task_factory: fn() -> SharedTask,
}
```

### ELF32 vs ELF64 (compile-time)

Use `#[cfg(target_pointer_width = "64")]` to select the right ELF structs. This is a compile-time concern, not runtime pluggable.

### Everything else is universal

ELF magic, LOAD segments, DYNAMIC parsing, DT_RELA/DT_RELASZ — all cross-platform.
