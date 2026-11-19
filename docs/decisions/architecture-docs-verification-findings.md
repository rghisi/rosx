---
status: decided
date: 2026-11-19
revisit-when: none
---

## Question
Did verifying the moved architecture prose against the tree find discrepancies?

## Chosen
Every backticked path and identifier in `docs/architecture/*.md` was checked against the tracked tree. Only naming drift remained: the prose names `rom/` and `task/` as directories while the kernel has `kernel/src/rom.rs` and `kernel/src/task.rs` (plus `task_manager.rs`); `Rescan`, `ReadChar`, `LIST_DIR`, `ATTACH` and `LIST_DEVICES` appear only as statements about names that no longer exist, and `ip_inject`, `ip_slirp` and `future_ownership` are integration-test file names. No claim about behaviour was found false, and no code was changed.

## Alternatives
Silently correcting prose or code: forbidden by the plan; discrepancies are recorded instead.

## Consequences
The module names stay as written in the prose until a later pass renames them.
