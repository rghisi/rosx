# Rationale documents

One file per low-level concern: why one piece of `unsafe` or assembly is shaped the way it is. Decisions (`docs/decisions/`) record choices and deferrals; rationale records the reasoning behind specific code. A rationale file may cite decisions; no decision is required to cite a rationale file.

A doc comment links a file with the exact form `docs/rationale/<slug>.md` (optionally `#anchor`). The `check-style` gate allows a `///` or `//!` block only on an item containing `unsafe`, `asm!` or `global_asm!` (or an `unsafe fn` / `unsafe impl`), only when it links an existing rationale file, and only up to 6 lines. Every file here must be linked from at least one doc comment. Plain comments inside `asm!`/`global_asm!` invocations stay allowed: assembly carries its register and calling-convention notes.

Template, one level-2 heading each:

```
## Why this is hard
## Invariants relied on
## What breaks if changed
## Evidence
## Status
```

`Evidence` names a test, a plan, or a hardware note. `Status` is `verified`, `unverified on hardware`, or `superseded by <file>`.
