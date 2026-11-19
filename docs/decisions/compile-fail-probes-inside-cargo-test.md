---
status: decided
date: 2026-11-19
revisit-when: the probe harness becomes costly to maintain
---

## Question
How is the handle type-shape proof (formerly `checks/compile_fail.sh`) run automatically?

## Chosen
A zero-dependency host integration test, `usrlib/tests/handle_kind_guards.rs`: per probe snippet it materializes a scratch crate under `target/` with its own lockfile, runs `cargo check`, and asserts a non-zero exit with `error[E0308]` plus one positive-control snippet that compiles. It runs inside `cargo test -p usrlib`, hence `cargo xtask test` and CI. The script is retired.

## Alternatives
Real `trybuild`: about 20 third-party dev-dependencies and `.stderr` files pinning rustc rendering, which churns on the pinned nightly.

## Consequences
The script previously mutated a tracked file and was run by nobody, so it could rot silently; that gap closes with the test.
