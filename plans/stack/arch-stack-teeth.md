# Plan 4 — Enforcement Teeth per Architecture (Umbrella)

> **Status:** DRAFT for review · **Date:** 2026-11-04 · **Code state:** `da8d54b`
> **Depends on:** plans 2 (contract, guard vocabulary, disposition path) and,
> for most leaves, plan 3 (the switch/entry hooks the arming rides on).
> **Nature:** this is not one change. It is one shared contract plus
> independent per-arch leaves; each leaf lands when its hardware exists.
> Leaf A is the only one executable on the current two ports.

## The tier gap plan 2 leaves open

Tier 1 (poison + switch-time checkpoint) detects overflow within one
scheduling quantum, attributes task + stack kind, but not the instruction —
and nothing faults a wild access that skips the guard. Every platform can
sharpen this with its own mechanism behind **one unchanged kernel-side
contract**.

## The contract (kernel-side, portable, defined by plan 2)

1. **Region registry:** the kernel can answer, for any address: is it inside
   task *X*'s guard range for stack *kind*? Backed by plan 2's invariant that
   every stack exists as a registered `[guard][body]` region, body power-of-two
   and size-aligned.
2. **One disposition entry:** `stack_fault(task, stack_kind, address)` → the
   plan-2 disposition (log with the address, terminate, notify completion
   future, machine continues). Every leaf routes here; leaves add the address
   to the log line, nothing else.
3. **Arch obligations:** (a) allocate stack regions honoring the alignment
   rules; (b) *arm* the platform mechanism if it has one; (c) wire the
   platform's fault vector into the disposition entry. Everything else stays
   in `kernel/` untouched — the AGENTS.md HAL rule in action.

## Leaf A — x86_64: unmapped guard-page arena (executable today)

**State today:** the MMU is on; the bootloader maps all RAM with large pages,
and `clear_nx_bits` (`arch/x86_64/src/cpu.rs:144-163`) even opens NX for heap
ELF execution. There is **no page-fault handler on either port** — a #PF
today dies. The blocker for guard pages is *granularity*, not an absent MMU.

**Work:**
1. Carve a dedicated virtual window for task stacks; build 4 KiB-mapped
   page-table pages for that window only (present: stack bodies; **absent: the
   guard slot below each body**). Leave all other mappings as they are.
2. Arena allocator for slots (power-of-two, size-aligned — contract already
   requires it; the FreeList can already `alloc` with alignment).
3. `#PF` handler (`interrupts.rs`): if CR2 falls in any registered guard
   range → `stack_fault(...)`. Otherwise: log + halt (strict improvement over
   the triple fault that happens today for *any* fault).
4. TaskStack allocation for stack kinds routes through the arena; Tier-1
   poison stays as belt-and-braces (it also guards the pages *around* the
   arena).

**Properties:** instruction-precise (CR2 exact), catches hardware interrupt
frames pushed at a full stack and giant-frame skips alike (a descent cannot
step over an absent page — probes not required at 4 KiB), **zero cost per
context switch** (flat address space; PTEs never change per task — this is the
guard-in-a-flat-space dividend), zero per-switch TLB work.

**Risk register:** page-table construction bugs are boot-fatal → land behind a
boot-time self-test (write probe into a scratch guard PTE, expect and handle
the fault once); NX interaction is neutral (absent ⇒ fault on any access,
execute included).

## Leaf B — x86_64: IST reserved stacks (NMI / DoubleFault)

Point the NMI and DoubleFault IDT entries at a dedicated IST slot backed by a
`StackKind::Reserved` TaskStack (guard attached, checked). This is the
mechanism's sweet spot: vectors that must run when the current stack is
unusable — it switches before the first stub instruction. **Explicitly not for
timer/keyboard**: IRQ-on-IST forces switch-at-trampoline with frame *migration*
(incompatible with plan 3's frame-stays model; its successor, not this).
MachineCheck optional. Prerequisite: none beyond plan 3's trampoline house
style for a reserved-stack exit (`iretq` from IST frame).

## Leaf C — ARMv8-M/v7-M: MPU guard regions (future arch port)

Standard MPU-OS configuration, and the industry precedent is exact:
**FreeRTOS + MPU = deferred switch (PendSV) + MPU stack guard regions** — the
pair plans 1 and 3 already established as this project's shape.

- Foreground MPU region per current task's guard: access denied **including
  privileged** → Data Access Violation with MMFAR = exact address → the
  disposition entry.
- Reprogram the guard region's base where plan 3's switch already loads the
  next kernel-stack top — same hook, a handful of CSR writes.
- Region constraints (power-of-two, aligned-to-size) are **already imposed by
  the plan-2 contract** — that is why that rule was adopted on x86 where it
  costs nothing.
- Cortex-M bonus: PendSV is the deferred model in hardware; plan 1's model
  ports as the PendSV handler.

## Leaf D — RISC-V: PMP guard entries (future arch port)

NAPOT PMP entry per guard range (`A=0xFF` pattern: naturally aligned
power-of-two — plan-2 contract fits again), RWX denied; load/store/access
fault → classify against registry → disposition. Trap entry already
software-saves everything: plan 3's record protocol is natively expressible
(no frame copy or stay question).

## Leaf E — m68k: notes for the feasibility-stage port

- **Dual stacks are native:** 68000+ keeps USP/SSP; supervisor mode switches
  SP by hardware, and the interrupt frame lands on SSP. Plan 3's
  software-switch protocol becomes a hardware mode-switch; the saved-user-SP
  record still works unchanged (the record protocol is SP-source-agnostic).
- Plain Amiga-class 68000 (no PMMU): Tier 1 only — already delivered by
  plan 2; document the tier in the port.
- 68851/68451 PMMU: Leaf-A-style guard translation at 4 KiB granularity;
  68010 address-space tables give coarse region checks. Both after the port
  exists.

## Leaf F — stack probes (all archs, conditional)

Guard-skip (a single frame larger than the guard) is closed today by the
disassembly large-frame gate inherited from
`plans/taskstack-extraction-plan.md`. **Trigger to enable:** if that gate ever
relaxes, enable compiler probes (`__rust_probestack` /
`-Z stack-size-probes`) so any descent touches the guard page/zone stepwise.
Not needed while the gate holds — recorded so the dependency is remembered in
both directions.

## Verification gates (per leaf)

1. Fault-injection negative test per arch: overflow → disposition with exact
   address in the log (CR2/MMFAR), machine survives, other tasks' data intact.
2. Leaf A additionally: boot-time self-test of the arena PTE machinery;
   survivor suite on the debug kernel; a wild-jump test
   (`call 0xdeadbeef`-class) still lands in the `#PF` log+halt path, not a
   triple fault.
3. Cross-cutting gate block (`plans/stack/README.md`) per touched port.

## Open questions for the review

1. Leaf A landing order vs plan 3: independent enough to interleave, but the
   #PF handler should land *before* plan 3 flips entries (plan 3's hard
   constraint 7 then faults precisely instead of checkpointing).
   Recommend: plan 3 → leaf A → revisit edge-case semantics from checkpoint to
   fault.
2. Arena placement: fixed VA window below the kernel's dynamic physical
   mapping (64-bit) — decide concrete window when designing leaf A; must not
   collide with bootloader mappings or ELF image heap allocations.
3. Should Leaf B's reserved stacks be per-CPU (future SMP) sized 2 now?
   (Recommend yes: one slot, array-of-2 capacity, cheap today.)
