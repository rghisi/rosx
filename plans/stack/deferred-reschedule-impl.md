# Deferred reschedule — implementation evidence

Branch: `deferred-reschedule`, forked from `origin/main` @ `ab0ea6a`.
Parent document: `plans/stack/deferred-reschedule.md`.

## Divergence from the plan draft

The plan was drafted at `da8d54b`. Before execution, PR #86 (`cf97d19`) had
already landed a partial deferral with the intent stored *after* the decision:
`preempt()` consulted `should_preempt()` and set `pending_preemption`, and
`apply_pending_preemption()` executed the switch at the timer-IRQ tail. The
implementation reshaped that onto the plan's design instead of adding onto it:

- `pending_preemption` became `need_resched` — an intent raised by wakers,
  never by the decision itself.
- `preempt()` + `apply_pending_preemption()` collapsed into
  `Kernel::maybe_resched()`: inert without intent, deferred (not cleared)
  while `preemption_enabled == false`, otherwise clears the intent and runs
  the decision body verbatim (`should_preempt()` → stamp `Preempted` →
  `switch_to_scheduler()`).
- `Kernel::mark_resched()` is null-safe on `KERNEL_PTR`: no execution state
  before boot, nothing to mark. This also keeps the services-registry unit
  tests (which reach `wake_tasks` without a `Kernel`) alive without test-only
  production surface.
- `SchedulerWakerUseCase::wake_tasks` raises the intent *after* enqueuing, so
  a consult never sees intent without the woken task on the ready queue.
- Both timer handlers: tick → EOI → `mark_resched()` → `maybe_resched()`.
- Both ports' `syscall_handler` wrappers consult after `handle_syscall`
  returns; no asm changes (the wrapper-tail switch is the mechanic
  wait-based syscalls already use).

## MLFQ quantum accounting (settled at review, confirmed by measurement)

`MlfqStrategy::should_preempt()` decrements `remaining_quantum` per call, and
`on_task_start` resets it on every pick. A syscall-exit consult therefore only
burns when the intent flag is set — a plain return is inert. A parked service
yields exactly one intent per wake window (its accept waiter is consumed by
the first delivery), so a sub-tick yield fires at the wake-marked consult
whose index equals the slice's remaining quantum (≤2 for a fresh slice, less
when a pending intent from the spawn traffic consumed one).

## Integration gate (`tests-integration/tests/ipc_wake_latency.rs`)

`ping burst` alternates eight sends to Ping and Random — both parked and
exclusive at burst time — between `WAKE BURST BEGIN` and `WAKE BURST SENT`.
The very first send wakes Pong, so by the accounting above the producer is
forced to yield within the first two intent-marked syscall exits and the
`PONG 64 matched ok` line lands between the markers. Tick-only scheduling
leaves every daemon line after the completion marker: the burst's send loop
runs in microseconds.

Measured discrimination: with the consult restored, the gate passes 5/5 in
isolation and in the full suite; with the x86_64 wrapper consult disabled and
a rebuilt kernel, the gate fails with the PONG line after the marker
(`pong at 44, begin at 11, sent at 28`).

First revision used BulkEcho as a wake target and proved flaky in the full
suite: BulkEcho is shared with the bulk app's boot traffic, and a send landing
while echo is mid-job finds no accept waiter and marks no intent. Exclusive
services only (Ping, Random) removed that interference class.

## Verification

- `cargo test -p kernel -- --test-threads=1`: 384 passed, including the six
  new decision-site tests in `kernel/src/kernel.rs`: inert consult, clearing
  consult with `Preempted` stamp, intent survival across a disabled window,
  `mark_resched` set/inert-before-boot, and intent via a registry-notify wake.
- `cargo xtask build` and `cargo xtask build x86_32`: both ports build.
- `cargo xtask check-isa x86_32`: kernel and 27 user ELFs 486-clean.
- Large-frame scan on `target/rosx/debug/rosx`: no `sub $>=0x1000` anywhere,
  none in the touched symbols.
- The first x86_32 gate run failed with the default sweep output under
  `ping burst`: the embedded ROM was stale because `cargo xtask build x86_32`
  had not re-run after the ping change. After the rebuild the gate passes
  repeatedly on x86_32 (`ROSX_ARCH=x86_32 cargo test -p tests-integration
  --test ipc_wake_latency`).
- `cargo xtask test` and both full integration suites (x86_64 and
  `ROSX_ARCH=x86_32`): green at branch head — see the suite tails recorded
  in the PR.
