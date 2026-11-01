# x86_32 IPC wake-path race (follow-up to the main merge, 2026-11-01)

**STATUS: FIXED 2026-11-01** on branch `x86_32-wake-race`. Root cause and
the full milestone evidence live in `plans/x86_32-wake-race-impl.md`.
The cure: interrupts stay masked in every kernel context except idle
`hlt` (`sti` removed from `swap_context` on both arches; x86_32 `int80`
forces IF=1 only in the saved eflags before `iretd`), plus a production
`IrqGuard` tripwire (`kernel/src/irq_guard.rs`) around the scheduler,
future registry, and mailbox manager. Verified: debug x86_32
`kstats_mem` 6/6 unguarded, both arches' full integration suites green,
`echo-stress` 30/30 boots clean.

## Symptom

A user task parked on an IPC reply future is not woken by a `send_message`
issued from a kernel task context. The sender completes
(`send_message` returns `Ok`); the parked receiver never resumes.

## Facts (measured 2026-11-01, QEMU 10.2.1, host x86_64)

- Harness `kstats_mem` on x86_32: pre-merge head `6599d0c` passes 6/6; the
  merged tree (main = `e3b764d`, protocol/ + x86-generic) fails 0/6 — same
  machine, alternating rounds.
- With temporary instrumentation: `mem` reaches its `wait_for_message` park;
  kstatsd logs accept → stats → `reply sent ok=true`; the reply is delivered
  into the IPC layer but `mem` is never rescheduled. The shell stays blocked
  waiting on `mem`; later keystrokes are never echoed (ps2d/inputd chain is
  collateral, not the fault).
- Release-profile x86_32 kernels pass; debug-profile kernels fail on the
  merged tree. Timing/layout-sensitive: every x86_32-relevant diff main
  brought is semantically neutral (port executor = identical asm under a new
  struct name; VGA/debug console = same design plus a `+0` offset; ANSI
  parser = same code; Cargo.lock adds only `x86-generic`).
- Pre-existing, not introduced by the merge: the same machine today shows
  echo-stress keystroke loss (~1/10 boots) on the pre-merge head, and it
  showed 1/10 on the M8-era tree once. The merge layout moved the
  `kstats_mem` instance of the same race from latent to near-certain.
- x86_64 has never shown this failure across the full suite on identical
  kernel code (hundreds of QEMU sessions).

## Prime suspects

1. `swap_context` (`arch/x86_32/src/cpu.rs`) ends with `sti; ret`: a task
   resumed mid-syscall runs the remaining kernel bookkeeping with IF=1, where
   a timer IRQ can preempt inside the same structures the resume path is
   mutating. x86_64 keeps IF=0 across the whole syscall via SFMASK, so the
   window exists only on x86_32.
2. The park-registration-vs-notify window in `FutureRegistry`/
   `IpcManager` for kernel-originated deliveries (`deliver_to_service`, and
   kernel-task `send_message` → user waiter wake), which the M2 `cli` at
   `int80_handler` entry covers for user syscalls but may not cover for
   kernel-task senders.

## Reproduction

- `ROSX_ARCH=x86_32 cargo test -p tests-integration --test kstats_mem -- --test-threads=1`
  on the merged tree (debug profile): fails ~100%; swap in pre-merge head to
  see 6/6 pass.
- Keystroke variant: `ROSX_KERNEL_ELF=target/rosx-i686/debug/rosx-x86_32
  python3 arch/x86_32/echo-stress.py 10 10` — occasional lost bursts.

## Suggested approach

M1/M2 discipline: contract-diff the x86_64 vs x86_32 wake paths (park
registration, notify, scheduler handoff, IF state at each step), instrument
counters for park/notify/wake, prove lost-wake, then fix the window (likely
an IF-discipline change in `swap_context`/idle path or a notify-atomicity fix
inside `FutureRegistry::notify`).

## Interim state

`kstats_mem` is guarded on x86_32 with a pointer to this document until the
race is fixed; the rest of the x86_32 suite passes on the merged tree.
