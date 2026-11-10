# Silent machine death under sustained IPC volume (2026-11-10)

**STATUS: OPEN — latent kernel bug, newly exposed by `fsbench`. Not caused by
the block-run batching work (reproduces through the same VFS path on code the
batching does not touch; the same silent-stall family already appeared as the
one-off 60 s `rose>` suite flakes on baseline builds).**

## Symptom

After a sustained single-connection VFS workload (order 2,000–3,000
request/reply transactions, reached ~30–90 s into a full `fsbench` run), the
whole guest goes silent at once:

- console output stops entirely — including a 1 Hz occupancy dump printed
  from the timer IRQ handler (present temporarily during the investigation),
  so the timer interrupt is no longer being serviced;
- QEMU keeps running: process alive, stderr empty, monitor socket openable;
- no panic banner, no `[IRQGUARD]` trip, no stack-watermark trip, no second
  boot banner (so not a triple-fault reboot).

The wedge point varies run to run — observed inside `ipc_send`, inside
`wait_for_message`, and between two adjacent prints — the same
corruption-spread signature the M2 probe produced for the 2026-11 wake race
(`plans/x86_32-wake-race-impl.md`).

## Window dependence

Reproduces only in the outer hypervisor's slow throughput windows (this host
is nested KVM, suite QEMU runs TCG — see `plans/block-read-batching-plan.md`
§8): 7/7 wedges while the window was bad, 0/11 across a later good stretch,
same binaries. Window indicators: `fsbench` control phase ~2,450 vs
~2,800 ops/s; `ata_identify` suite step ~2.9 s vs ~1.9 s.

## Facts measured (all instrumentation temporary and reverted)

- Resource pools are flat at the wedge and throughout the run: future arena
  ~21/1024, future waiters ~10, mailboxes 14/256, mailbox waiters 13, queued
  messages 0, data objects 0–3/256, connections 5/256, tasks 13/256. No
  leak, no saturation, no budget pressure anywhere in the IPC layer.
- Client-side step prints located the stop inside kernel calls, not the VFS
  server logic (vfsd never answered — but also never crashed).
- Extra console prints only moved the wedge point around; identical binaries
  passed a full run in a good window and failed 3/3 in a bad one.

## Ruled out

- Pool exhaustion / handle leaks (occupancy dumps above).
- vfsd panic or death (no banner; reply never sent).
- The W1–W5 window family of the 2026-11 wake race: structurally closed by
  the IF-masking discipline (`plans/x86_32-wake-race-impl.md` M3.1) and the
  `[IRQGUARD]` tripwire stayed silent. This wedge needs a new window class.
- Client heap corruption (wedge sits inside syscalls, not allocations).

## Prime suspects (next probe)

1. CPU spinning in kernel context with IF=0: the timer is never delivered
   again → total silence, QEMU alive, no reboot. Look for a loop reachable
   from syscall or IRQ tails that can spin when a queue invariant breaks.
2. A handler that wedges before its PIC EOI (interrupt-gate entry masks IF,
   so a spin inside any handler kills all further IRQs).
3. Run-queue corruption that strands the idle task: if the only runnable
   task is never `Idle`, the `sti`/`hlt` loop is never reached and the
   machine never services interrupts again.

## Forensics recipe (for the next bad window)

1. Build a throwaway `tests-integration/tests/wedge_forensics.rs`: spawn a
   `QemuSession`, run full `fsbench`, poll `content_since`, and after ~25 s
   of total console silence connect to
   `Monitor::connect(session.temp_dir().join("monitor.sock"))`.
2. Print `info cpus` (the `halted` flag distinguishes hlt from a spin) and
   `info registers` at t0 and t+6 s; identical EIP across the two dumps
   pins the stuck instruction.
3. Symbolize with
   `nm -n target/rosx-i486/release/rosx-x86_32 | ...` (nearest preceding
   symbol + offset is enough in the release build).
4. The `spawn_capturing` session already passes `-monitor unix:...` — no
   harness change needed.

## Consequences today

- The `fsbench` integration test runs the gated `fsbench /fsbrun 1`
  (control only) so CI stays deterministic; the read and directory phases
  remain available manually (`fsbench [dir] [phases]`, phases: 1 control,
  2 reads, 3 dirs).
- The fsbench-based before/after measurement of the block-run batching
  (B0′ vs F′) is deferred behind this fix; see
  `plans/block-read-batching-plan.md` §7.
