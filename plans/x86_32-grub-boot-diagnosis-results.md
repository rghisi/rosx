# x86_32 GRUB-Boot Triple Fault — Diagnosis Results

> **Status:** Diagnosis complete; root cause named and reproduced
> bidirectionally. Written 2026-11-02. Follows
> `plans/x86_32-grub-boot-diagnosis.md` (executed on branch
> `x86_32-grub-boot-diagnosis`, base `62c530d`, instrumented builds).
> **The fix itself is the separate approved change proposed in §5.**
> All diagnostics (boot.S sentinel dump, linker symbol, cure candidate)
> were throwaway and have been reverted; nothing speculative is committed.

---

## 1. Root cause (one paragraph)

The kernel hardcodes the code-segment selector `0x08` into every IDT gate
(`arch/x86_32/src/interrupts.rs`, `IdtEntry::interrupt_gate` and
`trap_gate_ring3`). Multiboot v1 does not define the selector values of the
loader's GDT — it only requires a flat memory model. QEMU's loader
(SeaBIOS) happens to place its flat 32-bit code descriptor at selector
`0x08` (so the kernel has always worked under `-kernel`), but **GRUB enters
the kernel with `CS=0x10`, `SS=0x18`, and GRUB's GDT descriptor `0x08` is
NULL** (D2 dump: `gdt1=0000000000000000`). The IDT is armed during
`Kernel::setup` but IF stays 0 until `Kernel::start` calls
`cpu.enable_interrupts()` — the `sti` at `Kernel::start+0x5e`. The first
timer IRQ (`v=0x20`, PIT already programmed by `enable_timer`) is delivered
at the instruction boundary immediately after that `sti` returns
(`pc=0x118013`, un-instrumented build). Delivering it requires loading
`CS ← 0x08` from the GDT; descriptor 1 is NULL, so the CPU raises
**`#GP(error_code = selector = 0x8)`**, then `#DF` (no double-fault gate),
then triple fault — `rose>` never prints because the kernel dies ~10 ms
after `[KERNEL] Starting`, inside the very first interrupt. The whole death
is synchronous, matching every observed fact: zero IRQ vectors delivered
before it, identical memory maps between modes, and the `sti`/swap_context
disassembly (the fat-pointer dispatch at the logged pc is
`call [ebx+0x10]` = `enable_interrupts` — a completely healthy call).

## 2. Evidence

### 2.1 D1 — static forensics supersede plan facts 4/5

The plan's "corrupted vtable pointer" reading was **wrong**:

* `0x205058` **is** the real `Cpu` vtable. It is referenced in the clean
  image at `KCONFIG+4 = 0x2050b4` (`.data`), as the vtable half of the
  `cpu: &dyn Cpu` fat pointer.
* Vtable layout (verified against `kernel/src/platform/cpu.rs` declaration
  order): `[+0]=drop(null) [+4]=size(0) [+8]=align(1) [+0xC]=setup
  [+0x10]=enable_interrupts(0x1012f6) [+0x14]=disable_interrupts
  [+0x18]=are_interrupts_enabled [+0x1C]=initialize_stack [+0x20]=swap_context
  [+0x24]=get_system_time [+0x28]=halt`.
* So the fault-site sequence is: `call [ebx+0x10]` → `sti` → `ret` →
  **`add esp,4` at `0x118013`** (the logged pc) → timer IRQ delivered right
  here → GP(8) → DF. No pointer was ever corrupt; the "vtable look-alike"
  conclusion of plan fact 5 inverted the truth — 0x205058 *was* the
  referenced table, and "landing on `enable_interrupts`" was not a
  coincidence, it was the intended call.

### 2.2 D2 — entry-state sentinel dump (the verdict)

Raw-asm debugcon dump at `_start` (pointer-walked in `ebx`, see §7),
same instrumented kernel, both boot modes:

| field | GRUB ISO | `-kernel` (SeaBIOS) | verdict |
|---|---|---|---|
| mbi pointer | `0x00010000` | `0x00009500` | mbi far below kernel image → **H1b eliminated** |
| CR0 / CR4 / EFLAGS | `0x11 / 0x00 / 0x46` | `0x11 / 0x00 / 0x46` | identical → **H2 (CR/EFLAGS form) eliminated** |
| `.bss` @`bss_start`, mid boot-stack, `kernel_end-16` | all zero | all zero | GRUB honors the zero-fill duty → **H1a eliminated** |
| **CS / SS** | **`0x10 / 0x18`** | **`0x08 / 0x10`** | **the divergence** |
| GDT base | `0x000010b0` (limit `0x27`) | `0x000cb2b8` | different GDTs |
| **GDT[1] (selector `0x08`)** | **`00000000_00000000` — NULL** | `0000ffff_00cf9a00` — flat CS32 | **root cause** |

Both loaders zero-fill, hand over an mbi outside the kernel image, and
agree on CR/EFLAGS state. The *only* boot-time difference that matters:
the value of the kernel code selector.

### 2.3 Exception fingerprint (identical, both modes, instrumented build)

```
GRUB:      0: v=20 e=0000 IP=0010:00118193   ← first IRQ, right at sti
           1: v=0d e=0008 IP=0010:00118193   ← #GP, error_code = failed CS selector
           2: v=08 e=0000                     ← #DF → triple
kernel+inj:0: v=20 e=0000 IP=0008:00118193   ← same, with SeaBIOS GDT[1] forced NULL
           1: v=0d e=0008 IP=0008:00118193
           2: v=08 e=0000
```
(The `IP` selector differing `0010` vs `0008` is the mode signature; the
pc — `Kernel::start+0x63` — and the error code are identical.)

## 3. Hypothesis scoreboard

| hypothesis | verdict |
|---|---|
| H1a — GRUB didn't zero-fill `.bss` | **Eliminated** (D2 sentinels zero under both) |
| H1b — GRUB mbi inside kernel BSS | **Eliminated** (mbi at `0x10000`) |
| H2 — inherited CR/EFLAGS bug | **Revised & confirmed in its true form**: the inherited state that kills is the **GDT/selector layout**, not CR bits. QEMU's GP error code 8 is the failed CS selector, not a MOV-to-CR marker. |
| H3 — IRQ/PIT timing | Eliminated (confirmed: the fault rides the first delivery) |
| plan fact 5 (corrupt fat pointer) | **False** (D1: intact, referenced vtable) |

## 4. Bidirectional reproduction (D4)

**Cause injection under `-kernel`.** Break at `enable_interrupts`
(0x101476), zero the two words of SeaBIOS's `GDT[1]`
(`base = *(u32*)sgdt_buf+8` → write `0,0` at `0xcb2c0`), continue.
Result: `v=20 → #GP(e=0x8) → #DF → triple` at `Kernel::start+0x63` —
the exact GRUB fingerprint (§2.3) — with nothing else changed. (Control:
injecting at the wrong offset — the `sgdt` base sits at `+2`, not `+4` —
boots healthy to `rose>`, proving the effect is the injection, not gdb.)

**Cure under GRUB.** Throwaway one-function change: both IDT gate builders
take `selector: current_code_selector()` (reads the live CS via
`mov {0:x}, cs`) instead of hardcoded `0x08`. Same ISO, same QEMU command
(`-cdrom … -boot order=d`) that faulted since the first ISO test now boots
**to `rose>`** with all eight boot-roster daemons, and its idle steady
state matches a healthy `-kernel` boot (incl. accumulated
`SYSTEM_TIME_MS ≈ 6.4 s` of real delivered timer ticks, where the faulting
build died on the first tick ≈ 10 ms after `start`). The `-kernel` path is
unaffected by the change (still boots to `rose>`).

Both directions pass ⇒ root cause proven, per plan §4/D4.

## 5. Fix proposal (the separate approved change)

Plan §5.1 ("zero `.bss` in `boot.S`") is **moot** — GRUB zero-fills
(D2). The failure is segment-table ownership, and the project rule is
*"the kernel owns its invariants"*. Ranked options:

1. **Install the kernel's own GDT at boot (recommended).** In
   `arch/x86_32/src/boot.S` `_start`: `cli` → load a private 3-descriptor
   GDT (`NULL`, `code 0x08` = `0x00CF9A00`, `data 0x00CF9200`) → `lgdt` →
   far-jump reload of `CS` → reload `DS/ES/SS` (+ `FS/GS` to the data
   selector) → set up stack → `call kernel_main`. Then the hardcoded
   `selector: 0x08` in `interrupts.rs` becomes true by construction for
   *every* loader, and the kernel stops depending on loader-segment
   persistence/limits entirely (foundation for TSS/double-fault/ring-3
   later). `sti` discipline unchanged; IDT init already runs after.
   Verified-compatible: SeaBIOS already hands exactly this layout, so
   `-kernel` behavior is bit-identical; GRUB becomes equal to it.
2. Minimal alternative (proven by the D4 cure, two lines): IDT gate
   selector := runtime CS. Rejected as *the* fix: it still trusts the
   loader's descriptor contents, limits, and persistence, and leaves
   future TSS/ring-3 work still needing option 1. (Keep as fallback if
   option 1 surprises us on real hardware.)
3. Multiboot2 migration: not needed for this defect.

`.bss` zero-fill in `boot.S` remains a nice loader-agnosticism nicety
(defense-in-depth, 4 instructions) but is explicitly **not** part of this
root cause and can ride along or not.

## 6. Regression gate (D5, part of the fix change)

`tests-integration/tests/iso_boot.rs` (new):

* Build the ISO in-process via the runner's GRUB image API (landed with
  #102 — this test depends on that branch; on current `main` the recipe
  is `arch/x86_32/build-image.sh`), skipping cleanly when
  `grub-mkrescue`/`xorriso`/`mformat` are not on `PATH` (the runner
  already exposes that probe).
* Boot `qemu-system-i386 -cdrom <iso> -boot order=d -display none
  -no-reboot -serial none -parallel none -debugcon <pipe>`, assert
  `rose>` within the suite's boot timeout. **`-boot order=d` is
  mandatory** whenever a dummy disk exists (pitfall #2 below).
* No `-d int` on green boots (1.1 GB/45 s class of failure).
This closes "only QEMU's loader ever tested us" permanently: any
selector/table regression fails CI on the GRUB path.

## 7. Environment hazard found during diagnosis (record so nobody re-learns it)

On this dev box, **QEMU 10.2.1 TCG mis-executes the byte-walk idiom
`mov al,[eax] / test al,al / jz / out 0xE9,al / inc eax / jmp`** in
multiboot kernels: only the first byte reaches the port; subsequent loads
return stale/foreign bytes (e.g. endless `0x72 'r'` — 6.7 MB flood — or an
`[EAX]`/`[EAX+ECX]` alternation, depending on register pressure), while a
`hlt`-parked snapshot shows the loop having "completed" correctly. Verified
with a 30-line standalone kernel: identical source, six loop shapes —

| walk | result |
|---|---|
| `mov al,[eax]` + `inc eax` (any decoration: nop, lea, call-boundary) | corrupted / early exit |
| `mov al,[eax+ecx]` (index form, base still EAX) | corrupted |
| **`mov al,[ebx]` + `add ebx,1`** | **perfect** |
| **`mov al,[edi]` + `inc edi`** | **perfect** |

A kernel with no port writes emits nothing, so it is guest-side execution,
not the chardev. Not diagnosed to QEMU internals here — **upstream-report
candidate** (repro: multiboot ELF, 1 MiB load, rodata string, the loop
above, `-debugcon file:`). For asm diagnostics meanwhile: walk pointers in
a register other than `EAX`, or load into a temp and `mov` into `AL`.
(The D2 table in §2.2 was produced with the safe `ebx`-walk form; every
number in it reproduced across rebuilds and both boot modes.)

Related trap from the same box (plan pitfall #2 reconfirmed): a master-IDE
dummy disk with an MBR signature hijacks CD boot unless `-boot order=d`.

## 8. Out-of-scope observation (pre-existing, mode-independent)

At full idle (every task parked, all `rose>`), `SYSTEM_TIME_MS` stops
advancing under **both** boot modes — the idle/halt path appears to park
with timer delivery stopped, so a woken-by-timer wake from full idle may
not happen. Identical under SeaBIOS and GRUB, predates this defect, and is
not part of this diagnosis; worth its own investigation thread.

## 9. Next steps

1. Commit this results document (this branch) — awaiting go-ahead.
2. Fix change (separate, milestone-by-milestone): own-GDT in `boot.S`
   (option 1), unit-safe by construction, + `iso_boot.rs` gate (§6),
   validated on `-kernel`, GRUB ISO, and the x86_64 suite (unchanged).
3. Optional: file the QEMU TCG hazard (§7) upstream with the mini-repro.
