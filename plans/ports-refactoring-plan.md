# Ports Segregation — Driving vs Driven

> **Status:** Pending (2026-10-07)
> **Scope:** `kernel/src/lib.rs` port definitions move to `kernel/src/ports/` (+ import updates in 7 kernel files). No changes to behavior, syscall ABI, or arch crates.
> **Platform focus:** x86_64; `rosx-i686` shares `kernel/` and must keep compiling untouched.
> **Rule:** pure structural move — behavior-preserving only. One commit per step; verify + confirm commit message before proceeding (AGENTS.md).

---

## 1. Context

All 8 ports currently in `kernel/src/lib.rs` are **driven** ports: capabilities the core
requests (`For*` traits consumed by `Scheduler`, `FutureRegistry`, `MailboxManager`,
`IpcManager`). The driving side of today's system — syscall entry, interrupt entry,
timer tick — is not trait-shaped at all (`handle_syscall`, `Kernel::enqueue`,
`Kernel::preempt` are plain functions called by arch) and becomes trait-shaped ports in
later refactors. Churn: ~20 import lines across 7 kernel files; neither arch crate
imports any port or `SwitchOutcome`, so `arch/x86_64` and `arch/x86_32` are untouched.

Conventions:

- `For*` naming is reserved for **driven** ports.
- Driving ports (added by the syscall refactor) are named per operation.
- `SwitchOutcome` is the contract type of `ForSwitchingTaskContext` → lives in `ports/driven.rs`.
- No `pub use` compat shims in `lib.rs`; every import shows its side explicitly.

---

## 2. Commit 1 — create `kernel/src/ports/`

- `ports/mod.rs` → `pub mod driving; pub mod driven;`
- `ports/driving.rs` → empty placeholder
- `ports/driven.rs` → move verbatim from `lib.rs`:
  - `ForWakingTasks` (`pub(crate)`)
  - `ForNotifyingFutures` (`pub(crate)`)
  - `ForCompletingExpiredTimers` (`pub`)
  - `ForSwitchingTaskContext` (`pub`) + `SwitchOutcome` (`pub`)
  - `ForReadingSystemTime` (`pub`)
  - `ForExpiringTimers` (`pub`)
  - `ForHandlingHardwareInterrupts` (`pub`)
  - `ForManagingTasks` (`pub`)
- `lib.rs` → remove the 9 definitions and the `use` statements that existed only for them,
  add `pub mod ports;`

---

## 3. Commit 2 — update imports (7 files)

| File | Lines |
|---|---|
| `kernel/src/scheduler/scheduler.rs` | 7–13 |
| `kernel/src/scheduler/mod.rs` | 9–10, 57, 70 |
| `kernel/src/kernel.rs` | 14–19 |
| `kernel/src/kernel_services.rs` | 12 |
| `kernel/src/future.rs` | 9 |
| `kernel/src/ipc/ipc_manager.rs` | 8 |
| `kernel/src/ipc/mailbox_manager.rs` | 8, 108 |

Rewrite `use crate::ForX` / `crate::SwitchOutcome` → `crate::ports::driven::…`.

---

## 4. Commit 3 — verification

- `cargo test -p kernel -- --test-threads=1`
- `cargo xtask build`
- `cargo build -p rosx-i686`

---

## 5. Follow-ups (separate efforts)

1. Keyboard subsystem → `ForKeyboardInput` into `ports/driven.rs`
2. Syscall dispatcher → first named-operation trait into `ports/driving.rs`
3. Kernel task orchestration extraction
