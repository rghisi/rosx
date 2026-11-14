# Phase 11 — CPU Guard, Page Tables, Long-Mode Entry (stage2-64 + `fake-kernel-64` smoke)

The x86_64 half of the plan. Everything 486-reachable stays exactly as it was; the 586-era
instructions (`cpuid`/`rdmsr`/`wrmsr`) exist only in the `stage-64` feature build and only behind
the CPU guard (decision D4).

## 1. Toolchain spike (first checkpoint of this phase — before any asm)

`fake-kernel-64` links at **VA `0xFFFF_8000_0010_0000` with `AT(0x0010_0000)`**, code model
kernel. There is no `-mkernel` rustflag; verify the actual recipe on the pinned nightly and
record the result in the commit: expected path is the `x86_64-unknown-none` target (already the
base of `arch/x86_64/rosx.json`) + linker script with `.-= HIGHER_HALF_OFFSET`-style `AT()`
sections + `-z max-page-size=0x200000` + LLVM kernel code model via
`-C llvm-args=-code-model=kernel` (or the current supported equivalent — **verify, don't
assume**; the constraint that matters is sign-extended-32-bit representable addresses, which
`0xFFFF_8000_xxxx_xxxx` satisfies). If the spike fails, the phase stops here for a decision
rather than improvising around it mid-assembly.

## 2. CPU guard (runs before any 586 instruction retires)

1. `EFLAGS.ID` (bit 21) set/clear probe → CPUID available? If not (real 486): print
   `[ERR] CPU does not support x86_64 Long Mode`, write `0x01` to `0xF4` (exit 3).
2. `CPUID 0x80000001.EDX[29]` (LM): if clear, same verdict line + exit 3.
3. The guard's failure text is the negative test's assertion — one canonical line, printed via
   the same serial path as everything else, and **no `hlt`-only hang**: diagnosability is a
   requirement (exit 3 is distinguishable from exit 1 triple-fault).

## 3. Page tables (numbers corrected: 6 pages, and PML4[256] aliases the PDPT)

Allocate **6 zeroed 4 KiB pages** above `initrd_phys_end`: `PML4`, `PDPT`, `PD[0..3]`.
`PD[i][j] = PRESENT|WRITABLE|PS` 2 MiB pages → identity `0..4 GiB` (2048 entries; `-m` capped at
1 GiB in tests but the map is honest to 4 GiB). `PDPT[k] = PD[k] | P|RW` for k<4.
`PML4[0] = PDPT | P|RW` (identity — carries the transition), `PML4[256] = PDPT | P|RW`
(higher-half window `0xFFFF_8000_0000_0000..0xFFFF_8001_0000_0000`). Both maps stay live at
kernel entry (frozen ABI, `00-boot-abi.md` §3). The page range enters the
`KernelAndModules` reservation (extends phase 9's span — `page_table_end`).

## 4. Long-mode switch (asm, `stage-64` module)

Load the 64-bit GDT per the ABI table (`0x08` L=1/D=0, `0x10` data, `0x18` user data, `0x20`
user code — STAR-compatible); `CR3 ← pml4_phys`; set `CR4.PAE`; `EFER.LME` via `WRMSR
0xC0000080` (set bit 8, preserve the rest — `RDMSR` first); set `CR0.PG`; `ljmp` 32-bit selector
to the trampoline, which lands in 64-bit mode, reloads `0x10` data, and (this phase only)
executes a **minimal** `fake-kernel-64` entry: the higher-half-linked image prints
`[FK64] alive` from long mode (proof: identity *and* higher-half fetch both work since the jump
target is a higher-half VA reached with the identity map live) and writes `0x10` → exit 33.
The full handoff is phase 12.

## 5. Verification

1. `lm-smoke` on `boot64.img` (`qemu-system-x86_64 -cpu qemu64`): `[FK64] alive` + exit **33**.
2. `lm-guard-486` (negative): `boot64.img` on `qemu-system-i386 -cpu 486`: the canonical
   `[ERR] … Long Mode` line + exit **3** — and exit code 3 (not 1) proves no triple-fault, the
   exit-code law earning its keep.
3. `lm-guard-nolm`: `boot64.img` on `qemu-system-x86_64 -cpu qemu64,-lm` → same verdict.
4. Audit wiring: new `check-isa` mode for `stage2-64` = the `--stages` deny-list **plus exactly**
   `{cpuid, rdmsr, wrmsr}` — the mode is allow-listed by name, not by blanket skip; `stage1`,
   `stage2-32` audits unchanged and green; `fake-kernel-64` exempt (x86_64 artifact).
5. `stage2-32` continues to pass its verbatim deny-list audit and all 32-bit rows (the feature
   build split is proven honest both ways).

**Pass criteria:** rows 1–5 green; commit.
