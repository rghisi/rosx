# Rules Ledger

Rules and facts routed to their sinks through the `retro` skill: gates, `docs/architecture/`, `AGENTS.md` and `docs/todo/`. This file tracks them; the rule text lives in its sink. Format and review criteria: `.opencode/skills/retro/references/ledger.md`.

    <date> | <rule or fact, shortened> | <sink: file> | <evidence: plan and friction entry> | <expected effect> | <status>

2026-11-20 | `cargo xtask check` runs its tests stage with QEMU integration excluded; integration evidence comes from `cargo xtask test` or an explicit `ROSX_KERNEL_ELF` run | AGENTS.md Commands | plans/boot-disk-directories-impl.md friction "believed `cargo xtask check` covered the QEMU suites" | a green gate is never mistaken for QEMU proof | active
