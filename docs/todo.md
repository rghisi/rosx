# Investigation Ideas

Backlog of architectural investigations to evaluate when there is time. Neither is committed work; both were discussed during the kernel-owned-handle run (PR #58, 2026-10-11) and deferred.

## 1. trybuild-lite: run the handle type-shape proof inside `cargo test`

Replace the manual `checks/compile_fail.sh` gate with a zero-dependency host integration test (e.g. `usrlib/tests/handle_kind_guards.rs`).

**Why.** `checks/compile_fail.sh` (added in `ca5bd4f`, plan step 18, requirement R13) proves the three `usrlib` handle kinds are compile-time distinct (`UserConnectionHandle` ↛ `UserFutureHandle`, no binding into `ipc_send_value`, no fabrication from a raw word). Verified facts from PR #58: `usrlib` builds and tests on the host, and the probe properties are pure library-surface type properties — the bare-metal pipeline is not required to check them. Today the script mutates a tracked file (`apps/shell/src/shell.rs` + trap/restore) and is **run by nobody automatically**: CI (`cargo xtask test --skip-integration`) never invokes it, so it can silently rot between manual runs.

**Sketch.** One host integration test that, per probe snippet, materializes a scratch crate under `target/` (outside the workspace, own lockfile — the mechanism trybuild itself uses), spawns `cargo check`, and asserts exit != 0 with `error[E0308]` in stderr (keep the substring semantics; format-drift-immune on nightly). Plus one positive-control snippet that must compile. Runs inside `cargo test -p usrlib` → `cargo xtask test` → existing CI Unit Tests job. Then retire `checks/compile_fail.sh` and note the guard in AGENTS.md test section.

**Alternative considered.** Real `trybuild` — rejected for this repo: ~20 third-party dev-deps and `.stderr` files that pin rustc rendering (re-bless churn on the pinned nightly toolchain).

## 2. rust_arkitect: architecture tests for the crate/module graph

Evaluate [`rust_arkitect`](https://crates.io/crates/rust_arkitect) (ArchUnit/phparkitect-style rule DSL over parsed `use` relationships) as an automated guard for the structural invariants currently written as prose in AGENTS.md/README.md and checked by hand-run greps.

**What it can and cannot express (conclusion from the PR #58 discussion).**

- Can express: R11 dependency rules (`usrlib -> system` only, `kernel -> system` only, …), R2-style "system names no domain types", layer rules, cycles. Parse-level, so re-export laundering can slip through; never typechecks, so it cannot replace idea 1 — the two are orthogonal oracles.
- Cannot express: R13 token-carrier discipline (no layout-constant duplication, no bit arithmetic in usrlib), type opacity — those stay with compile-fail probes + greps + review.

**Caveats found during evaluation.** `rust_arkitect` is 0.3.x and young, but has the workspace-aware API (`Project::from_current_workspace`, `rules_for_crate`) that fits this workspace shape. The sibling `arch_test_core` / `cargo-archtest` looks like the wrong fit: 0.1.5 published 2026-09-27, AGPL-3.0, pulls the rust-analyzer syntax front-end (`ra_ap_syntax`), and its `ModuleTree::new("src/lib.rs")` reads single-project-oriented.

**Zero-dependency middle path (possibly sufficient).** A `cargo xtask check-arch` subcommand: `cargo metadata` allow-list of deps per crate (catches `system` ever gaining a dependency) + the R2/R5 absence greps. The RosX crate graph is ~5 edges and Cargo itself already refuses illegal arrows; the ArchUnit DSL earns its dev-dep only if module-level accretion *inside* `kernel/` becomes a real decay mode — that is the trigger condition to revisit this.

## Related open gap (decided, not yet done)

Neither idea is wired in today, so `checks/compile_fail.sh` remains a manual gate; at minimum it deserves a CI invocation and an AGENTS.md mention until idea 1 lands.
