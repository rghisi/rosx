# Phase 8 — ELF32 Loader + `fake-kernel-32` (smoke form)

The loader learns to place a kernel. **ELF32 only at this phase** — the ELF64 parsing arm is
deferred to phase 11/12 where its caller (the long-mode handoff) lives: an unreachable parser arm
would be exactly the dead production surface repo law forbids.

## 1. Loader (`stage2/src/elf32.rs`)

Read through `rxfat` in chunks into the bounce window; validate, then place:

- `e_ident[0..4] == 7F 'E' 'L' 'F'`, `EI_CLASS == 1`, `EI_DATA == 1`, `e_machine == 3` (`EM_386`);
  anything else is `[ERR] elf class/machine` + exit 3 — `stage2-32` boots `EM_386` or refuses,
  no silent leniency.
- Iterate `e_phoff`/`e_phentsize`/`e_phnum`; for each `PT_LOAD` with `p_memsz > 0`:
  require `p_paddr >= 0x100000` and reject any span overlapping the sub-1-MiB reservation zone or
  another segment (overlap = `[ERR] elf layout`, exit 3 — never load into the IVT);
  `p_filesz` bytes copied file→physical, `p_memsz − p_filesz` zeroed (`.bss`);
  track `kernel_phys_start` (min) and `kernel_phys_end` (max, **aligned up to 4 KiB**).
- Record `e_entry`; sanity-check it lies inside a loaded `PT_LOAD`'s span.
- Print `[ELF] arch=X86_32 entry=0x… phys=0x…..0x…`.
- The kernel bytes stay resident where placed; the phase-7 initrd staging move to
  `kernel_phys_end`-relative happens in phase 9.

## 2. `fake-kernel-32` (crate, smoke form)

- Workspace member on `rosx-boot-i486`; `linker.ld` places everything at `0x0010_0000`,
  `static` relocation model, no paging assumptions; links `ros-boot-common` for serial +
  debug-exit; `_start` is `extern "C" fn(eax_magic: u32, ebx_ptr: u32) -> !` — at this phase it
  just prints `[FK32] alive`, writes `0x10` to `0xF4`, halts. (Full form takes over in phase 10;
  the parameters are already ABI-shaped so its signature never lies.)
- Audited by the **486 mnemonic allow-list** (same bar as every kernel/user ELF) in the
  `bootloader` CI job; `check-isa`'s app enumeration extended with a bootloader-ELF list.
- Packager: `xtask boot-image` compiles it and places it as `KERNEL.ELF` on the volume.

## 3. Verification

1. `elf32-load` on `boot32.img` (`-m 32M`, `-cpu 486`): stdout shows `[ELF] arch=X86_32
   entry=0x00100000 phys=0x00100000..0x0010xxxx` **then** `[FK32] alive` **then** exit **33** —
   stage2 places the image, a hand-rolled `jmp` entry (the full handoff is phase 10) runs it, and
   the exit-code law closes the loop.
2. Negative rows: `KERNEL.ELF` replaced with an ELF64 file → `[ERR] elf class`, exit 3;
   ELF32 with a `PT_LOAD` at `p_paddr = 0` → `[ERR] elf layout`, exit 3.
3. Host unit tests for the parser: header/segment validation matrix, overlap refusal, phys-end
   alignment arithmetic (pure module, fake device backend, same pattern as phase 7).
4. All prior rows + both audits (deny-list bins, allow-list `fake-kernel-32` ELF) green.

**Pass criteria:** rows 1–4 green; commit.
