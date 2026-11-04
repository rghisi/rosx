# Stack Plan Family — Sizing, Segregation, Enforcement

> **Date:** 2026-11-04 · **Code state:** `da8d54b`
> **Origin:** Stack sizing assessment and MMU-free segregation discussion
> (2026-10/11). Predecessor design: `plans/taskstack-extraction.md` (delivered —
> `TaskStack` now owns the buffer, opening this whole agenda).

## The problem family

One `TaskStack` per task (2048 words: 16 KiB resident on x86_64, 8 KiB on
x86_32, `kernel/src/task_stack.rs:4`) carries all four cost centers serialized
in a single buffer: app frames, syscall trampoline + dispatch chain, hardware
interrupt frames, and context-switch frames. There is no user stack in the ELF
loader; overflow is silent heap corruption (the failure class of
`plans/game-triple-fault-fix.md`). All decisions here are deliberately
**ring-0, no-MMU**: segregation via ownership discipline, enforcement via
software detection, with the contract shaped so hardware teeth (IST, guard
pages, MPU regions) bolt on per-arch later without touching `kernel/`.

## The plans, in dependency order

| # | Plan | One-line outcome |
|---|---|---|
| 1 | `deferred-reschedule.md` | One single place where a forced switch happens: the exit check. Fixes IPC wake latency (≤10 ms → immediate) on the way. |
| 2 | `stack-kinds-and-poison-guards.md` | Per-kind sized stacks with attached poison guards; overflow becomes a logged task kill instead of the triple-fault class. Answers the sizing question. |
| 3 | `dual-stack-entry-trampolines.md` | Per-task user + kernel stacks; software-switched entry/exit trampolines; ABI-correct ELF initial stack. The structural core. |
| 4 | `arch-stack-teeth.md` | Umbrella for the per-arch enforcement leaves: x86 guard-page arena, IST reserved stacks, ARMv8-M MPU regions, RISC-V PMP, m68k notes, stack probes. |

Dependencies: **1 → 3** (the trampoline inherits the single switch site) and
**2 → 3** (the deferred-frame design in plan 3 is only safe once guards bound
overflow). **2 is independent of 1** and may land in parallel. Plan 4 leaves
land after plan 3 for the arch hooks they need, except none of plan 4 blocks
plans 1–3.

## Cross-cutting regression gates (every plan)

1. `cargo xtask test` (and `cargo test -p kernel -- --test-threads=1`).
2. QEMU survivors on the **debug** kernel: `snake_survives_idle`,
   `tetris_survives_idle` (the gate block of
   `plans/taskstack-extraction-plan.md` Step 4).
3. Debug disassembly large-frame gate (`sub $>=0x1000,%rsp` scan) on every
   touched constructor — the hard constraint inherited from the taskstack
   extraction: no giant value-semantics temporaries, ever.
4. Both ports build: x86_64 via `cargo xtask build`, x86_32 via
   `cargo build -p rosx-x86` from `arch/x86_32/`.

## Budget facts anchoring the numbers

- Task ceiling: 256 slots (`kernel/src/task_manager.rs:8`).
- QEMU RAM: 128 MiB default on both ports (no `-m` in either `run.sh`).
- x86_64 target already sets `disable-redzone: true` (`arch/x86_64/rosx.json:13`).
- Interrupts are masked across syscall and handler bodies on both ports
  (`SFMask` at `arch/x86_64/src/cpu.rs:52`); today's `preempt()` runs after PIC
  EOI (`arch/x86_64/src/interrupts.rs:104-113`, `arch/x86_32/src/interrupts.rs:159-163`).
