---
status: decided
date: 2026-11-19
revisit-when: none
---

## Question
What did the 2026-11-19 audit of `AGENTS.md` find?

## Chosen
Context for `plans/agents-md-slimdown.md`:
- `AGENTS.md` was 56 KB; eight paragraphs of 2-5 KB were design records duplicating code, plans and tests.
- Content was duplicated: boot plan in four places, FAT boot disk in three, x86_32 integration notes in two; the directory tree duplicated `Cargo.toml`, the xtask list duplicated `cargo xtask help`.
- Rules stated in prose but unenforced: "no comments" (186 non-doc `//` lines and 54 doc-comment lines existed), `#[expect]` over `#[allow]` (one `#[allow(dead_code)]` in `arch/x86-generic/src/vga_buffer.rs`), compile-fail probes run by nobody, dependency arrows checked by hand-run greps, the `ROSX_KERNEL_ELF` dev-profile footgun.
- The incremental-approval workflow contradicted hands-off operation.

## Alternatives
None; this is a findings record.

## Consequences
Each finding maps to a step of the plan: gates in `cargo xtask check`, prose in `docs/architecture/`, workflow in `AGENTS.md`.
