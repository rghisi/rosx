# IPC Target Architecture — Hexagonal (Ports & Adapters) + DDD

> **Companion to:** `plans/ipc-hexagonal-refactor.md` (the *how* / execution plan).
> This file is the *what* / *why*: the reference model the code is being reshaped toward.
> Keep the two in sync. If the execution plan changes the target, update this too.

---

## 1. One-paragraph summary

RosX IPC is a **generic broker** in the kernel. It knows *names* (services) and *plumbing*
(mailboxes + async completion), but never *what a service means*. Services are **user-space
apps** that `bind` a name and serve clients. The broker's logic lives in an **application
layer** (`IpcBroker`) that depends only on four **ports** (traits in the `system` crate).
Concrete **adapters** in the kernel (`MailboxManager`, a `CompletionEngine` over the
`FutureRegistry`, the `Scheduler`, and a printer) implement those ports. This is the
hexagonal layout: the center (broker + domain value types) is platform-free and
host-testable; the ring (syscall driver, usrlib driver, future/scheduler/clock adapters)
plugs in the real OS.

---

## 2. Why this shape

- **Testability.** The broker's whole job is orchestration (bind/connect/send/receive state
  machine + async completion). Today it can only be exercised with `services()` booted.
  With ports injected, it runs on the host against fakes — the single biggest win for
  "ease of testing."
- **Extensibility (the actual goal).** Adding a filesystem or networking service must *not*
  require touching the broker. In this model, a new service = a new user-space app that
  `ipc_bind`s a name. The broker is already generic; the refactor just makes that true by
  construction (it can't grow a special case without violating "depends only on ports").
- **Maintainability.** One seam. The broker no longer reaches into `FutureRegistry`/`Scheduler`
  by name; it asks a `CompletionEngine` to "complete this future" and a `TaskWaker` to "wake
  these tasks." Swapping the future/scheduler internals later won't ripple into IPC logic.

---

## 3. The hexagon for IPC

```
┌──────────────────────────────────────────────────────────────────────────────────┐
│                              ADAPTERS (the ring)                                  │
│                                                                                   │
│   INBOUND (drivers)                    OUTBOUND (driven)                          │
│   ┌─────────────────────────┐          ┌──────────────────────────────────────┐   │
│   │ syscall dispatcher       │          │ MailboxManager ── MessageQueuePort   │   │
│   │ (kernel, Ipc* arms)      │          │ CompletionEngine ── over FutureReg   │   │
│   │ usrlib Syscall (user)    │          │ Scheduler adapter ── TaskWaker       │   │
│   │                          │          │ kprint/panic ── IpcErrorReporter     │   │
│   └────────────┬────────────┘          └───────────────────▲──────────────────┘   │
│                │  calls broker methods                     │ implements ports     │
│                ▼                                           │                       │
│   ┌─────────────────────────────┐                          │                       │
│   │  APPLICATION LAYER          │   ◄──────────────────────┘                       │
│   │  IpcBroker                   │   depends only on PORTS (traits)                │
│   │   • bind / connect /         │                                                 │
│   │   • send_to_server/client    │   ◄── value types ──┐                            │
│   │   • receive_*_async          │                     │                            │
│   │   • disconnect               │                     │                            │
│   └──────────────┬───────────────┘                     │                            │
│                  │ uses                                 │                            │
│                  ▼                                      ▼                            │
│   ┌─────────────────────────────┐     ┌────────────────────────────────────────┐   │
│   │  DOMAIN (pure value types)  │     │  PORTS (traits) — defined in system::ipc │   │
│   │  IpcMessage, IpcConnection, │     │  MessageQueuePort / CompletionEngine /    │   │
│   │  IpcBinding, IpcServerBinding,│    │  TaskWaker / IpcErrorReporter            │   │
│   │  error enums, ServiceName   │     └────────────────────────────────────────┘   │
│   └─────────────────────────────┘                                                  │
└──────────────────────────────────────────────────────────────────────────────────┘

Services live OUTSIDE the hexagon, in user space:
   app "FS"  ── ipc_bind("FS") ──► serves ipc_connect("FS") clients
   app "NET" ── ipc_bind("NET") ─► serves ipc_connect("NET") clients
   (random_gen_server is today's template)
```

**Key rule the hexagon enforces:** the APPLICATION LAYER + DOMAIN reference only PORTS and
value types. They never name `services()`, `FutureRegistry`, `Scheduler`, or
`alloc::boxed::Box`. Adapters are the *only* things that may.

---

## 4. DDD mapping

| DDD term | In RosX IPC |
|----------|-------------|
| **Bounded context** | "IPC" — self-contained; the value types + broker + ports form it. Other contexts (scheduler, memory, filesystem) touch it only through the ports / the broker's public methods. |
| **Aggregate(s)** | `IpcBinding` (a bound service name + its mailbox handle), `IpcConnection` (server↔client mailbox pair). `IpcBroker` is the aggregate root guarding them (all mutation goes through it, preserving invariants like "no double bind" and "send only on a live connection"). |
| **Value objects** | `IpcMessage { data, connection_handle }`, `IpcBindingHandle`, `IpcConnectionHandle`, the error enums. Immutable/`Copy`, no identity. |
| **Domain service** | (Currently none — the broker *is* the application service. If per-service policy ever moves in-kernel, it would live here.) |
| **Application service (use-cases)** | `IpcBroker` — orchestrates ports to run the use-cases `bind`, `connect`, `send`, `receive`, `disconnect`. No business *rules* beyond the broker's own invariants; it knows nothing of what a "RANDOM"/"FS" message is. |
| **Repository / ports** | `MessageQueuePort` (mailbox storage), `CompletionEngine` (async completion), `TaskWaker` (task wakeup), `IpcErrorReporter` (side-effect reporting). |
| **Anti-corruption layer** | `usrlib::syscall` ↔ kernel `syscall` dispatcher. They translate the raw `usize`/pointer/`pack()` ABI into `IpcMessage`/handles/`FutureHandle` — isolating the domain from the ABI. |
| **Ubiquitous language** | *service* (a bound name), *binding* (the handle for a service), *connection* (a client↔that-service pair), *mailbox* (a queue behind a handle), *message* (`data` + which connection it's on), *receive* (async → a future), *client* (whoever `connect`ed), *server* (whoever `bind`ed). |

**User-space service = the unit of extension.** The broker's invariants are intentionally
small so that *meaning* stays in the app: `random_gen_server` decides what a received
message produces; a future `FS` app decides how a path maps to a response. The broker only
guarantees delivery.

---

## 5. Ports (full reference)

All four live in `system/src/ipc.rs` (per locked decision #1). Handle types are the existing
`collections::generational_arena::Handle`; `MailboxHandle` and `FutureHandle` are the existing
aliases. `TaskHandle` origin is resolved (it is the `collections` `Handle`) — see the execution plan **Q1**.

```rust
// ── OUTBOUND: mailbox queue storage ─────────────────────────────────────────
// Implemented by: kernel MailboxManager
pub trait MessageQueuePort {
    fn create_queue(&mut self) -> MailboxHandle;
    fn drop_queue(&mut self, handle: MailboxHandle);
    fn push(&mut self, handle: MailboxHandle, message: IpcMessage);
    fn pop(&mut self, handle: MailboxHandle) -> Option<IpcMessage>;
    fn has(&self, handle: MailboxHandle) -> bool;
    // If the mailbox→future waiter list stays on the queue side (plan Q2, preferred):
    fn attach_waiter(&mut self, mailbox: MailboxHandle, future: FutureHandle);
    fn drain_waiters(&mut self, mailbox: MailboxHandle) -> Vec<FutureHandle>;
}

// ── OUTBOUND: async completion over the future registry ─────────────────────
// Implemented by: kernel CompletionEngine (wraps services().future_registry)
pub trait CompletionEngine {
    fn register_message(&mut self, message: IpcMessage) -> FutureHandle;
    fn register_pending_message(&mut self) -> FutureHandle;
    fn register_error_message(&mut self, error: IpcReceiveError) -> FutureHandle;
    fn complete(&mut self, future: FutureHandle, message: IpcMessage);
    fn wake(&mut self, future: FutureHandle);
    // (attach_waiter/drain_waiters live on MessageQueuePort if Q2 = "queue owns waiters")
}

// ── OUTBOUND: wake blocked tasks ────────────────────────────────────────────
// Implemented by: Scheduler adapter (thin; Scheduler::wake_tasks already exists)
pub trait TaskWaker {
    fn wake_tasks(&mut self, handles: Vec<TaskHandle>);
}

// ── OUTBOUND: side-effect reporting (keeps the broker side-effect-free) ─────
// Implemented by: a kprint/panic adapter
pub trait IpcErrorReporter {
    fn report_send(&self, error: IpcSendError);
    fn report_bind(&self, error: IpcBindingError);
    fn report_connection(&self, error: IpcConnectionError);
}
```

**Design notes:**
- `CompletionEngine.complete` does the `downcast_mut::<IpcMessageFuture>()` today done in
  `notify_waiters`. `wake` does `FutureRegistry::notify` (→ `Scheduler::wake_tasks`).
- The broker's `receive` on an empty queue: `register_pending_message` + `attach_waiter`;
  on a missing handle: `register_error_message`. These map 1:1 to today's three branches —
  the point is the broker no longer knows *how* a future is stored/woken.
- `TaskWaker` may be folded into `CompletionEngine::wake` (which already reaches the
  scheduler). Keeping it separate is cleaner for fakes; decide in Step 3.

---

## 6. Invariants the target must preserve (mirror of execution plan §3)

1. Syscall ABI (numbers, arg layout, `Box::into_raw` / `pack()` returns) unchanged → `usrlib`
   and apps need no edits.
2. Observable `bind/connect/send/receive/disconnect` behavior identical, including:
   empty receive ⇒ *pending* future that completes + wakes its task when a message arrives;
   missing handle ⇒ *error* future; `bind` creates a server mailbox that lives until
   shutdown (`disconnect` only removes the client mailbox).
3. Broker file contains no `services()` and no `alloc::boxed::Box` for completion.
4. No behavior fixes smuggled in (no new congestion handling, no new error variants).

---

## 7. How to add a new service after this refactor

A service is a **user-space app**. Template = `apps/random_gen_server`:

1. Create the app crate (excluded from the workspace, like `random_gen_server`).
2. In `_start`, `Syscall::ipc_bind("NAME")` (e.g. `"FS"`, `"NET"`).
3. Loop:
   ```
   fh = Syscall::ipc_receive_from_client(binding);
   if let IpcMessage(Ok(msg)) = Syscall::wait_future(fh) {
       reply = handle(msg.data);              // app decides the meaning
       Syscall::ipc_send_to_client(msg.connection_handle, reply);
   }
   ```
4. Clients do `Syscall::ipc_connect("NAME")` / `ipc_send` / `ipc_receive` / `wait_future` /
   `ipc_disconnect` (see `apps/shell/src/shell.rs::random()`).

**Nothing in the broker, ports, or `system` value types changes** to add a service. That
"does nothing need to change in the center" property is the acceptance test for this
architecture: if adding `FS` ever requires editing the broker, the hexagon has a leak.

> **Payload note:** payloads stay `usize` in this effort. Filesystem/networking will need
> richer payloads (paths, buffers). When that work starts, introduce a `MessagePayload`
> abstraction as a *separate, later* plan (locked decision #3 defers it). The broker/ports
> are already decoupled, so that change will be localized to `IpcMessage` + the two drivers.

---

## 8. Open items (gated in the execution plan)

- **Q1** `TaskHandle` on `TaskWaker` — ✅ resolved: `TaskHandle` is `collections`' `Handle` (`kernel/src/task.rs:10`), so alias it and keep **all four** ports (incl. `TaskWaker`) in the pure `system` crate. No circular dep.
- **Q2** Who owns the mailbox→future waiter list (preferred: the queue side).
- **Q3** Rename `IpcManager` → `IpcBroker` (lean yes).
- **Q4** Optional second template service ("echo") in the docs step.
