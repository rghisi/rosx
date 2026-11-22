# Rules Ledger

Rules and facts routed to their sinks through the `retro` skill: gates, `docs/architecture/`, `AGENTS.md` and `docs/todo/`. This file tracks them; the rule text lives in its sink. Format and review criteria: `.opencode/skills/retro/references/ledger.md`.

    <date> | <rule or fact, shortened> | <sink: file> | <evidence: plan and friction entry> | <expected effect> | <status>

2026-11-20 | `cargo xtask check` runs its tests stage with QEMU integration excluded; integration evidence comes from `cargo xtask test` or an explicit `ROSX_KERNEL_ELF` run | AGENTS.md Commands | plans/boot-disk-directories-impl.md friction "believed `cargo xtask check` covered the QEMU suites" | a green gate is never mistaken for QEMU proof | active
2026-11-21 | era-rehearsal tool facts: mtools `@@` is bytes not sectors; the chain's `[ELF]`→`[BOOTINFO]` burst prints within ~150 ms after a ~3–4 s ELF-load window (step 100 ms there); `fpu_registers` passes by raising `no FPU`; `record_start` broken by the server-venv NumPy ImportError | arch/x86-86box/docs/gotchas.md | plans/x86_32-era-retirement.md friction log (2026-11-21 rehearsal) | the V-step execution re-derives no tool quirk | active
2026-11-22 | check-style surveys `git ls-files` only: new sources count toward the unsafe ratchet and rationale links only after `git add` | AGENTS.md Rules enforced by check | plans/elf-load-at-syscall-impl.md friction M2+M3 "check-style reported 302→292 while program_memory.rs was untracked" | no gate surprise from untracked new files | active
2026-11-22 | run `cargo xtask test`/`verify` one at a time: the QEMU suites share `target/` and contend | AGENTS.md Toolchain notes | plans/elf-load-at-syscall-impl.md friction M4 "x86_32 suite started alongside the live x86_64 one" | one kill and rerun never repeated | active
2026-11-22 | capture long-suite output to a file with its exit code; a truncated tail is not a verdict — if the output was truncated, the suite never ran | AGENTS.md Toolchain notes | plans/x86_64-carrier-swap.md friction M1+V "mid-run-dead verify and a truncated `cargo xtask test` both looked like passes" | a dead suite is never mistaken for a passing one | active
