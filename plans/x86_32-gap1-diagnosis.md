# Gap 1 diagnosis — x86_32 keystroke stall (M1)

> **Status:** Complete — stall link identified and measured. Written 2026-10-31 on the
> `x86_32-parity-plan` branch (baseline `a1068e6`), per `plans/x86_32-parity-plan.md` §2,
> protocol D1 (contract diff) + D2 (instrumented run). Measured observations marked **[O]**.

## 1. The contract (from `cf97d19`, x86_64 reference)

Invariant: *interrupts are disabled across park-registration and restored by the arch exit
path; preemption is applied only at IRQ exit.*

How each platform edge satisfies it on x86_64:

| Step | x86_64 |
|---|---|
| syscall entry | `syscall` instruction loads RFLAGS masked by SFMASK → **IF=0** for the whole `syscall_handler_entry` + `handle_syscall` |
| park sequence | `Kernel::wait_future` (`kernel/src/kernel.rs` 206–214): completion-check → `block_current_task` → `register_waiter` → `switch_to_scheduler` — all run with IF=0, atomic vs IRQ |
| park exit | `swap_context` (`context_switching.S` 4–41): `cli` … `sti; ret` — IRQs re-enabled only after the context is switched away |
| return exit | `syscall.S` 59–63 (`cf97d19`): `or r11, 0x200; popfq` — caller resumes preemptible |
| preemption | `preempt()` only sets `pending_preemption`; `apply_pending_preemption()` switches at timer IRQ exit |

The wake protocol this relies on: `FutureRegistry::notify` (`kernel/src/future/mod.rs` 196–200)
removes and wakes the registered task waiters; a notify with **no registered waiters wakes
nobody**, and a later `register_waiter` on an already-notified future never fires. So the
check→block→register→switch window **must** be atomic against IRQs; that atomicity is supplied
by the arch's interrupt-window control, not by the kernel.

## 2. The violation (x86_32)

`arch/x86_32/src/interrupts.rs`:

- IDT entry `0x80` is a **trap gate**, `type_attr: 0xEF` (type 0xF) — trap gates do **not**
  clear IF. `swap_context` (`cpu.rs` 126–147) ends `sti; ret`, so every task runs with IF=1,
  and `int80_handler` (`cpu.rs` 163–185) never executes `cli`.

Consequence: `handle_syscall` — including the entire `wait_future` park sequence — runs on
x86_32 with **IF=1**. Both IRQ classes are live inside the window:

1. **Lost wake.** Keyboard IRQ → `deliver_to_service` → mailbox `push_back` →
   `notify_waiters` → `FutureRegistry::notify` fires while the receiving task is between the
   completion-check and `register_waiter` → notify finds no task waiters → task registers and
   switches away on an already-notified future → blocked forever.
2. **Mid-park preempt switch.** Timer IRQ → `preempt()` + `apply_pending_preemption()` →
   `switch_to_scheduler()` *from IRQ context, mid-syscall*: the task is already `Blocked`
   (`block_current_task` ran) so the scheduler does **not** requeue it, but its waiter
   registration is incomplete → the task is stranded mid-syscall and its mailbox future is
   orphaned. Every later message to that future completes with a silent (zero-waiter) notify.

Suspect 2 from the plan (IRQ-context handoff duration) and suspect 3 (idle/halt wake) are not
indicted: EOI-before-deliver and halt behavior match x86_64, and the 100 Hz tick keeps waking
idle. The stall is entirely suspect 1.

## 3. Measured evidence **[O]** (QEMU 10.2.1, this machine)

Baseline reproduction (throwaway harness: boot `qemu-system-i386 -kernel target/rosx-i686/debug/rosx-x86`,
wait `rose>`, monitor `sendkey` 10 letters at 450 ms):

```
echoed 0/10   echoed 0/10   echoed 0/10   echoed 8/10   echoed 0/10
```

— O6 confirmed (far worse than the plan's runs).

Instrumented run (throwaway counters: `keys` = scancodes delivered to ps2d mailbox,
`parks` = park sequences entered, `wakes` = notifies with waiters, `silent` = notifies with
zero waiters, `iqpark`/`pdpark` = timer IRQs seen / preemptions fired inside an open park
window), dumped every second from the x86_32 timer handler:

```
[DIAG] keys=20 parks=5 wakes=1 silent=1 iqpark=2 pdpark=2   (frozen across 3 s while keys kept climbing)
```

Reading: all 20 scancodes (10 keys × make+break) reached the kernel; park entries stopped at 5
— one task froze **inside** its park after `pdpark` preempt-switches in the open window; the
chain's future was orphaned, so the next delivery produced exactly one silent notify and every
later key queued in the ps2d mailbox with nobody to wake. `wakes` never advanced again.

This is shared-code-agnostic: the same kernel wake code runs correctly on x86_64 precisely
because the arch closes the window. **D3 escalation gate not triggered** — fix is arch-layer.

## 4. Fix specification (lands in M2)

`arch/x86_32/src/cpu.rs` `int80_handler`: `cli` on entry.

- Park exit: `swap_context` already ends `sti; ret` (identical shape to x86_64).
- Return exit: `iretd` pops the caller's saved EFLAGS (IF=1 — tasks always run preemptible,
  see `swap_context` `sti`), restoring the window exactly as x86_64's `or r11, 0x200` does.

Residual, shared with x86_64 and out of scope here: kernel `FunctionTask`s (`kstatsd`) call
`Kernel::wait` directly with IF=1 — the same window exists there on **both** arches as a
pre-existing latent condition of the reference platform. Not part of O6 (kstatsd is not in the
keystroke chain); if it is ever hardened, it is a kernel-level plan of its own.
