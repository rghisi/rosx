# IPC and futures

## Invariants

- Futures are notification-driven (time, task-completion, depth-one waitset aggregates); nothing polls on the preemption cycle.
- A waitset completes when any attached leaf future does, woken by the child-to-waitset cascade in `FutureRegistry::notify`.
- Every future's owner, parent links, and group membership live in the one arena entry keyed by its slot — there are no parallel tables to disagree — and `reclaim_group` reads only that group's index, not the arena.
- The kernel never interprets message payloads; IPC messages signal and data rides the connection out-of-band through write/read/release tokens.

## Design

`future/` + `ipc/` — **notification-driven** futures (time, task-completion, depth-one waitset aggregates) and a mailbox IPC manager (bind/connect/send/receive); kernel-originated event delivery (`deliver_to_service`) wakes accepts on bound services; a waitset completes when any attached leaf future does, woken by a child-to-waitset cascade inside `FutureRegistry::notify`; no polling on the preemption cycle. The task-completion future carries the terminated task's exit status — see the exit-status channel.

`ipc/` data family — messages signal; data rides the connection out-of-band via write/read/release tokens; the kernel never interprets message payloads.
