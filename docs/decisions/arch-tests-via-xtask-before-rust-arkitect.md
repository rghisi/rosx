---
status: open
date: 2026-11-19
revisit-when: module-level decay inside kernel/ appears
---

## Question
Adopt `rust_arkitect` (rule DSL over parsed `use` relationships) for structural invariants?

## Chosen
Default for now: `cargo xtask check-arch`, a `cargo metadata` allow-list of dependencies per crate. The crate graph has about five edges and Cargo already refuses illegal arrows.

## Alternatives
- `rust_arkitect` 0.3.x: workspace-aware API (`Project::from_current_workspace`, `rules_for_crate`); parse-level only, so re-export laundering can slip through and it cannot typecheck; cannot express token-carrier discipline or type opacity, which stay with compile-fail probes and review.
- `arch_test_core` / `cargo-archtest`: 0.1.5, AGPL-3.0, pulls `ra_ap_syntax`, single-project oriented; wrong fit.

## Consequences
Revisit only when accretion inside `kernel/` modules becomes a real decay mode.
