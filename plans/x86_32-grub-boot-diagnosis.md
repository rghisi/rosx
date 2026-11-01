# x86_32 GRUB-Boot Triple Fault — Diagnosis Plan

> **Status:** Plan (diagnosis not yet executed). Written 2026-11-01.
> **Branch base:** `main` at `62c530d` (#98). Kernel bits are identical
> between `main` and the runner-unification branch (#102) — only build
> tooling differs — so every fact below holds on both. Found by #102's
> ISO gate: the **first time this kernel ever booted under GRUB**.
> **Method:** every fact was verified 2026-11-01 on the dev box (paths,
> commands, toolchain: `grub-mkrescue`, `xorriso`, `mtools`, QEMU).
> Commands this plan cannot prove are marked **`unverified`**.

---

## 1. Symptom & Goal

### Symptom (repro)
```bash
cargo xtask image x86_32          # via runner lib on #102; via build-image.sh on main
timeout 30 qemu-system-i386 -cdrom target/rosx-i686/release/rosx-x86_32.img \
    -boot order=d -display none -debugcon stdio -no-reboot -serial none -parallel none \
    -d int,cpu_reset -D exc.log
```
Last debugcon line `[KERNEL] Starting`; QEMU exits rc=0. `exc.log`
holds exactly two exceptions since reset: `#GP error_code=0x8` at
TB-pc `0x118013`, then `#DF`, then `Triple fault`. **Zero timer-IRQ
vectors delivered.**

### Goal
- Name the root cause with bidirectional reproduction (same failure
  made to happen under `-kernel`, or the GRUB path made to pass by the
  one identified state difference).
- Produce a fix proposal + a regression gate (ISO-boot integration
  test). The fix itself is a separate approved change, as are all
  changes here — this plan is diagnosis until the root cause is a
  statement, not a guess.

### Non-Goals
- No speculative kernel fixes before the D2 verdict.
- No boot-protocol redesign decisions here (Multiboot2 migration is a
  fix *option* recorded in §7, decided only if cheaper fixes fail).
- No CI changes; the CI artifacts workflow already builds the ISO.

## 2. Verified facts (2026-11-01)

1. **Boot-medium matrix for the same release kernel ELF:**

   | Invocation | Result |
   |---|---|
   | `-kernel <elf>` (no disks) | boots to `rose>` ✅ |
   | `-kernel <elf>` + dummy master IDE disk | boots ✅ |
   | `-kernel <elf>` + ATAPI `-cdrom <the ISO>` | boots ✅ — **ATAPI device presence exonerated** |
   | `-cdrom <iso>` ± disk, `-boot order=d` | GP→DF→triple after `[KERNEL] Starting` ❌ |

   (Pitfall, verified: a dummy master disk with an MBR signature
   hijacks BIOS boot order away from the CD unless `-boot order=d`.
   The first cdrom "silent" failure was this, not the kernel.)
2. **Trimmed memory maps are identical** between the two boot modes
   (`[MEMORY] Region: 0x216be0-0x7fe0000; Total: 125 MB`). The
   allocator's input is not the differentiator.
3. **Exception timing:** the GRUB death is *synchronous* — the `-d int`
   log contains zero delivered IRQ vectors before the GP, while a
   healthy `-kernel` boot delivers thousands of `v=0x20` ticks. Not an
   interrupt-timing race.
4. **Symbolized fault site:** `0x118013` = `<kernel::kernel::Kernel>::start + 0x63`
   — the return site of `call *0x10(%ebx)`, a **fat-pointer dynamic
   dispatch**. Source shape of `Kernel::start` (`kernel/src/kernel.rs:154`):
   `services().task_manager.borrow()` chain, then `cpu.enable_interrupts()`,
   then `self.cpu.swap_context(state_pointer, null_mut(), sp)` — a
   `&dyn Cpu` call with three arguments, matching the disassembly
   (two stack args + self).
5. **The vtable pointer is corrupt:** at fault, `EBX=0x205058` was read
   via `movl 0x4(%esi), %ebx`. Verified against the clean ELF image:
   **no word anywhere in the image's loaded ranges equals `0x205058`** —
   no static fat pointer references it. (The bytes there happen to look
   like a code-pointer table — `*0x10` lands on
   `<X86_32 as Cpu>::enable_interrupts` at `0x1012f6` — a coincidence,
   and a trap: `nm` nearest-symbol attribution called it
   `MULTIPLEXED_OUTPUT+0x28`. Verified no vtable symbol exists within
   0x100 of it.) Execution therefore diverged inside a garbage call;
   the `#GP(0x8)` fingerprint (QEMU raises error-code 8 for
   reserved-bit MOV-to-CR) is consistent with random bytes decoding
   into such an instruction and is **not** independent evidence of a
   CR-write bug in the kernel.
6. **Where the corrupted path lives:** `Kernel::start` touches (a) the
   `services()` task-manager RefCell statics and (b) the `Kernel`
   struct, which `kernel_main` allocates **on the boot stack — and the
   boot stack is 64 KiB of `.bss`** (`arch/x86_32/src/boot.S`). Both
   live inside the **zero-fill region of PT_LOAD @0x205000: FileSiz
   0xa54, MemSiz 0x11be0 → ends exactly at `kernel_end = 0x216be0`**
   (`arch/x86_32/linker.ld`, `readelf -lW`). `.bss` includes the
   RefCell borrow flags, `static CPU`, and task-table storage.
7. **Zero-fill is the loader's duty.** Multiboot v1 spec §3.1: "The
   boot loader must zero the BSS." QEMU's multiboot loader honors
   this; the kernel has **never been booted by any other loader until
   this ISO test**. This is the exact axis along which the two boot
   modes can differ with byte-identical ELF segments.
8. `boot.S` `_start` performs no `cli` and forwards `eax`/`ebx`
   (magic/mbi-pointer) to `kernel_main`. `swap_context`
   (`arch/x86_32/src/cpu.rs` global_asm) begins with `cli`; `start()`
   executes `sti` immediately before calling it.
9. `parse_memory_map` reads the raw mbi pointer directly
   (`arch/x86_32/src/main.rs:113`) — the kernel never copies the mbi,
   so where GRUB placed it is currently unknown (D2 measures it).
10. Diagnostics cost note (verified): `-d int` on a **healthy** boot
    wrote **1.1 GB in 45 s**. Crash captures must stay bounded
    (≤ 20 s or until exit) and be trimmed; never leave `-d int`
    unbounded on a booting system.

## 3. Hypotheses

Ranked; all agree on the mechanism "runtime corruption of a `.bss`-
resident fat-pointer path before/during `Kernel::start`":

- **H1a — GRUB did not zero-fill `p_memsz > p_filesz`.** `.bss` statics
  hold GRUB-era residue; the first code to depend on zero-init
  semantics (RefCell borrow flags / task tables / copied `&dyn Cpu`)
  reads garbage. Fits: sync death, identical maps, `-kernel` fine.
- **H1b — GRUB placed the mbi inside the kernel's BSS range**
  (reservation ending at the file-image end 0x205a54 rather than
  memsz end 0x216be0, then writing mbi + mmap buffer there). Fits the
  same evidence; also fits that mmap/trim hid whatever mmap entries
  marked that region.
- **H2 — CR/EFLAGS inherited-state bug** (the classic "GP(8) means
  MOV-to-CR reserved bit"). Demoted by fact 5: execution had already
  diverged into a garbage call, so the faulting instruction is
  whatever junk bytes decoded to. Kept only if D2 sentinels come back
  clean.
- **H3 — IRQ/PIT timing under GRUB.** Effectively eliminated by fact 3
  (zero IRQs delivered before death).

H1a and H1b share one discriminator (D2) and differ in fix.

## 4. Experiment ladder (cheapest + most discriminating first)

### D0 — Static forensics recap (done, this plan)
Clean-image bytes at 0x205058 recorded; no references to it; vtable
symbol absence verified; boot-stack/`.bss`/PT_LOAD layout established.

### D1 — Identify the Cpu vtable's true address (10 min, no code)
`nm` gives `<X86_32 as Cpu>::swap_context = 0x101260`,
`initialize_stack = 0x1012b0`, `enable_interrupts = 0x1012f6`. Search
the clean image for the table containing `0x101260` — that table's
address is the real Cpu vtable; record it. Success criterion: a table
`[size, align, drop?, methods…]` whose entries match the trait's
method set. Value: D2 prints the *runtime* fat pointer; the delta
against the static truth becomes readable.

### D2 — Entry-state sentinel dump (the verdict experiment; throwaway diagnostics)
Temporary, **uncommitted** edit to `boot.S`/`kernel_main` printing, as
the very first debugcon writes:
- the mbi pointer (`ebx` at `_start`);
- `CR0`, `CR4`, `EFLAGS` at `_start` (H2 gate);
- 16 bytes at three chosen statics: a `services()`/task-table RefCell
  static, `static CPU`'s vtable slot inside the `KCONFIG` rodata pair,
  and a `.bss` address at mid-boot-stack (never yet written at entry).
Boot both modes; diff table.
**Verdicts:**
- bss sentinels zero under `-kernel`, non-zero under GRUB → **H1a**
  (if residue is generic GRUB data) or **H1b** (if bytes contain the
  mmap entry shape / `0x2BADB002`/physical addresses, or the printed
  mbi pointer lands inside `[0x205a54, 0x216be0)`).
- mbi pointer inside `[0x204a64, 0x216be0)` → **H1b** confirmed.
- sentinels clean and CRs/EFLAGS identical → H2 gate opens; escalate
  to D3.
Pitfalls: debugcon writer must be raw `out 0xE9` asm (nothing
allocator/RefCell-dependent); keep prints ≤ ~120 bytes total; boot
with a ≤ 20 s timeout, no `-d int`.

### D3 — GDB truth (if D2 is inconclusive)
`qemu-system-i386 -cdrom <iso> -boot order=d -s -S` +
`gdb target/rosx-i686/debug/rosx-x86_32` (build the debug-profile
ISO: `cargo xtask image x86_32 --debug`; note addresses shift between
profiles — re-derive). Break on `Kernel::start`; inspect the `self.cpu`
fat pointer *before* the call; step the corrupt read to its source
write. Precedent for this path exists: `arch/x86_64/gdb-init.gdb`,
`arch/x86_64/DEBUG_SETUP.md`.

### D4 — Bidirectional confirmation (after root cause is named)
- Reproduce under `-kernel`: inject the identified state (e.g., a
  debugger write of GRUB-observed residue into the bss sentinel region
  before continuing) → expect identical GP→DF→triple.
- Cure under GRUB: the §7 fix candidate applied → expect `rose>` from
  the same ISO. Both directions must pass before the fix is proposed.

### D5 — Regression gate design (part of the fix change, listed here)
New `tests-integration/tests/iso_boot.rs`: build the ISO through
`x86_runner::create_grub_image` (already a library API), **skip
cleanly when `grub-mkrescue`/`xorriso`/`mformat` are not on PATH**
(the runner already exposes exactly that probe shape), boot
`-cdrom … -boot order=d` headless, `expect_output("rose>")`. This
closes the class of "only QEMU's loader ever tested us" permanently.

## 5. Fix candidates (for the follow-up change; choice depends on D2)

1. **Kernel-side self-defense (recommended default): zero `.bss` in
   `boot.S` before `kernel_main`**, after safely evacuating anything
   the kernel needs that GRUB wrote into the zero region (the mbi, if
   H1b: copy mbi→small `.data` buffer or early stack region *first*,
   then zero the rest of bss excluding the live stack window; or zero
   in two passes around the stack). This makes the kernel loader-agnostic
   and matches the project's rule that the kernel owns its invariants.
2. If only H1b: relocate nothing and instead copy the mbi at the
   earliest point and treat the bss range as reserved — weaker; does
   not fix loader zero-fill assumptions.
3. If D2 forces GRUB-specific protocol work: **Multiboot2 migration**
   (also the runner plan's §6 north star; MBI2 is GRUB's native path
   with a tag-parsed memory map). Bigger; decide only if 1–2 fail.

## 6. Rules for this diagnosis
- All diagnostics live uncommitted or on a throwaway branch; nothing
  speculative lands on `main`; commit messages carry no prefix.
- Kernel code style rules (no comments, minimal `unsafe`, safe wrappers)
  bind any fix, and the fix is its own milestone-by-milestone change.
- QEMU `-d int` runs: bounded time, bounded output (fact 10).
- Trust static forensics over `nm` nearest-symbol attribution (facts 5,
  and the vtable look-alike at 0x205058, both burned this session).
- Logged `pc` is TB-granular (fact 4/TD): confirm faulting instructions
  against disassembly of the exact TB, or gdb.

## 7. Risks & traps

| # | Trap | Mitigation |
|---|---|---|
| 1 | `-d int` fills the disk on healthy boots (observed 1.1 GB/45 s) | bounded timeouts; capture only on crash boots; grep, don't read whole |
| 2 | Dummy master disk hijacks CD boot order (observed: "silent" ISO run) | `-boot order=d` whenever a bootable dummy disk coexists with `-cdrom` |
| 3 | Coincidental pointer-shaped bytes (observed twice) | require a referenced table (D1), not look-alikes |
| 4 | Profile shift between debug/release changes addresses | pin one profile per experiment; rebuild the matching ISO (`--debug`) for gdb |
| 5 | mbi pointer print order: anything touching RefCell/statics before the dump can consume the corruption | D2 prints via raw asm only |

## 8. Success criteria
1. A one-paragraph root-cause statement referencing concrete D2/D3
   evidence.
2. Bidirectional reproduction (D4): cause injectable under `-kernel`,
   cure demonstrable under GRUB.
3. Fix proposal (one candidate from §5) + the D5 integration test
   sketched to the same standard as this plan; both land as a separate
   reviewed change.
