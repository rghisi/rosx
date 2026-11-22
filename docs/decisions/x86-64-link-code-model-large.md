---
status: decided
date: 2026-11-22
revisit-when: rustc gains a supported way to express a non-sign-extended high window, or the boot ABI's window base changes
---

## Question
The chain maps the kernel at virtual `0xFFFF800000100000` (the +2 TiB window
base `0xFFFF800000000000`). Which LLVM code model can the kernel target spec
use, and what does the linker script owe it?

## Chosen
`"code-model": "large"` in `arch/x86_64/rosx.json`. The `kernel` and `medium`
models address data through the 32-bit sign-extended range; the window's low
32 bits (`0x00100000`) do not sign-extend to the window address, so their data
relocations cannot hold it (`R_X86_64_32 out of range`). `large` reaches every
address through `movabs` at the cost of wider encodings — acceptable for this
kernel. `fake-kernel-64` links with the same spec (one file, one truth).

Consequence for both `linker.ld`s (kernel and fake kernel): the large model
splits functions and data into the `.ltext/.lrodata/.ldata/.lbss` orphan
families, so the scripts collect them inside their `AT(. - 0xFFFF800000000000)`
output sections. A script that does not collect them leaves the orphans to
rust-lld's default placement: they keep `p_paddr == p_vaddr` (outside every
loadable physical range) unless they happen to land inside a preceding
AT()'d section's region — which is why the real kernel initially survived
(boot.S keeps `.text` non-empty) while `fake-kernel-64` linked unloadable.
Explicit collection is the invariant; the ELF audit (`every PT_LOAD at
p_paddr = p_vaddr - 0xFFFF800000000000`) is its check.

## Alternatives
`"code-model": "kernel"`: fails to link the real kernel (data relocations).
Relink at a sign-extending higher-half base (Linux-style `-2 GiB`): rejected —
the window base is frozen in `plans/bootloader/00-boot-abi.md` and already
validated by the stage2/fake-kernel corpus.

## Consequences
`arch/x86_64/linker.ld` owns `.got` explicitly too (no RW orphans left to
luck); `-z max-page-size=0x100000` keeps VMA−LMA a constant 2 MiB-aligned
offset. Boot-test rows `lm-smoke` and `real-kernel-boot64` guard the shape.
