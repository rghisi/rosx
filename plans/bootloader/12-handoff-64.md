# Phase 12 — Higher-Half Handoff + `fake-kernel-64` (full form) — Completion

The frozen ABI gets its 64-bit consumer: `BootInfo` is translated to higher-half addresses, the
kernel is entered in long mode with the register contract satisfied, and the fake kernel proves
both mappings, the treaty, and the payload — one number checked four ways.

## 1. Stage-2 side (ELF64 arm — its caller is finally real)

- **ELF64 loader** (mirror of the phase-8 module for class 2): `EI_CLASS == 2`,
  `e_machine == 62` (`EM_X86_64`); `PT_LOAD`s placed at **`p_paddr`** (the `AT()` physical
  addresses), zero-fill `.bss`, same bounds/overlap refusals; `e_entry` is a **higher-half VA**
  — validate the top bits (`== 0xFFFF`) or refuse (`[ERR] elf entry`).
- `stage2-64` boots ELF64 or refuses; `stage2-32` refuses ELF64 (already tested phase 8).
- `BootInfo` assembly: same content as the 32-bit path, then **before** the jump convert
  `memory_map_ptr` and `initrd_addr` to `phys + HIGHER_HALF_OFFSET`; `BootInfo` itself is
  written 16-byte aligned in the `0x30000` window (identity-mapped, reachable both ways — the
  pointer handed over is the higher-half alias).
- Handoff: `RSP = 0x7F000 + HIGHER_HALF_OFFSET` (the reserved stack page, now reached through
  the direct map), `RAX = BOOT_INFO_MAGIC`, `RDI = &BootInfo` (higher-half), 64-bit data
  selectors per the ABI table, `jmp e_entry` (higher-half VA — the PML4[256] alias is what makes
  the target fetchable; this instruction *is* the proof of the higher-half map).

## 2. `fake-kernel-64` full form

`extern "C" fn(rax: u64, rdi: *const BootInfo) -> !`:

1. Prove higher-half residency: the entry label's own address ≥ `HIGHER_HALF_OFFSET`, and `RSP ≥
   HIGHER_HALF_OFFSET`; `rax == BOOT_INFO_MAGIC` before dereference; struct
   `magic`/`version`/`arch == X86_64`/`boot_method` asserted through `system::boot_info`
   accessors (dogfooding rule).
2. **Dual-map check:** read one byte of the memory map through the higher-half
   `memory_map_ptr` **and** through its `− HIGHER_HALF_OFFSET` identity alias, assert equality —
   both PML4 entries are live exactly as the frozen ABI promises.
3. Walk + validate the map (sorted/non-overlap/count), as the 32-bit twin does.
4. CRC32 the initrd through the higher-half `initrd_addr`; emit exactly:
   ```text
   kernel loaded
   arch: x86_64
   initrd_crc32: 0xXXXXXXXX
   ```
   `0x10` to `0xF4`; the epilogue `0x01`-exit-if-return rule as in the 32-bit twin.

## 3. Verification

1. `handoff64` (`qemu-system-x86_64 -cpu qemu64`, `-m 128M`): `[INITRD] … crc32=X` (stage2),
   then `kernel loaded` / `arch: x86_64` / `initrd_crc32: X`, exit **33** — **four-way** CRC
   agreement: host seed == stage2 log == kernel log == re-verified placement bytes.
2. `handoff64-512m`: `-m 512M` variant.
3. Negative rows still green: `lm-guard-486`, `lm-guard-nolm` (phase 11), ELF32-on-64 refusal
   row added here.
4. Full `cargo xtask boot-test` — every checkpoint from every phase — green ≥ 3 consecutive
   runs.

## 4. Plan completion (the overview §8 checklist, asserted here)

1. `boot-test` full suite green in CI (new `bootloader` job) on both binaries and the negative
   guards.
2. Every artifact audit-clean under its §6 policy; zero warnings across all build surfaces.
3. Existing boot stack provably untouched: full unit + integration suites pass unmodified at
   the final commit (`cargo xtask test` + both `ROSX_ARCH` integration runs).
4. `arch/x86-bootloader/README.md` complete: every artifact buildable and every checkpoint
   runnable by hand, era-context notes included.
5. ABI frozen; follow-on stubs (real-kernel integration, carrier swap, rom.img-as-initrd,
   era-hardware) recorded in `00-overview.md` §9 and acknowledged — no silent carries.

Commit subject at completion: bare imperative, e.g. `Prove the custom x86 BIOS bootloader
end to end in QEMU`.
