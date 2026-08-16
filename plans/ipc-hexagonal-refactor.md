# IPC Refactor — Hexagonal Architecture + DDD (Ports in `system`)

> **Status:** Planning (not started)
> **Scope:** IPC subsystem only
> **Platform focus:** x86_64 only (do NOT touch `arch/x86_32`; kernel/`system` changes apply to it for free and it must keep compiling, but it is not the target of this work)
> **Payload:** keep `data: usize` for now (no payload abstraction in this effort)
> **Companion doc:** `plans/ipc-architecture.md` — the "to-be" reference for the target model. This file is the *execution* plan (how to get there, step by step).

---

## 1. Purpose

RosX's IPC will become a **core OS feature** and the *extension point* for OS services (filesystem, networking, …). Today the IPC manager is a working kernel-internal object that is tightly coupled to kernel globals (`services()`, `FutureRegistry`, `Scheduler`) and, in a few places, to the *runtime* via `alloc::boxed::Box`. As a result its logic is very hard to test or reuse, and there is no clean seam for adding services.

This refactor reshapes IPC into a **Hexagonal (ports & adapters) + DDD** layout, keeping behavior **byte-for-byte identical** at every step, so that:

- the IPC **application logic** depends only on **ports** (traits), not kernel globals;
- it becomes **unit-testable on the host** (no boot, no real scheduler/clock/tasks);
- **services are user-space apps** that `bind` a service name and serve clients (the kernel is a dumb, generic **broker** — it never learns what a service *means*).

### Locked decisions (do not re-litigate)

| # | Decision | Choice |
|---|----------|--------|
| 1 | Where ports (trait boundary) live | **`system` crate** (pure, `no_std`, host-testable). The kernel *implements* the ports. |
| 2 | Services model | **User-space services.** Kernel = generic broker (name registry + mailbox plumbing only). Filesystem/networking are *apps* that `bind` a name. |
| 3 | Message payload | **Keep `usize`** (`IpcMessage.data`). No `MessagePayload` abstraction here. |
| 4 | Domain-model cleanup (rename/move structs) | **Skipped** in this effort (may be a later, separate plan). |
| 5 | Scope / platform | **IPC only.** x86_32 not touched (must still compile as a side effect). |

---

## 2. As-Is Snapshot (what exists today)

This is the ground truth an agent must work from. Verify against the code before starting — line numbers may drift.

### 2.1 Workspace & crates

- Workspace root `Cargo.toml`: members = `collections`, `system`, `kernel`, `usrlib`, `arch/x86_64` (bin name `rosx`), `arch/x86_32` (bin `rosx-x86`), `apps/shell`, `apps/dummy`. Excluded (built separately): `apps/hello_elf`, `apps/random_gen_server`, `apps/snake`, `apps/tetris`, `apps/conway`. Edition 2024, `panic = "abort"`.
- `kernel` is **`no_std`**; allowed deps are workspace crates `collections` + `system` + `lazy_static` only. **No new third-party deps may be added to `kernel` or `system`.**

### 2.2 The IPC pieces

**`system/src/ipc.rs`** — shared value types + the async wrapper (this crate is the kernel↔user boundary; its public API must keep working for `usrlib` and `arch`):
- `IpcConnectionHandle = Handle`, `IpcBindingHandle = Handle` (both `collections::generational_arena::Handle { index, generation }`).
- Errors: `IpcConnectionError { ServerNotFound, ConnectionCannotBeEstablished }`, `IpcBindingError { AlreadyBound }`, `IpcSendError { ConnectionNotFound, ConnectionCongested }` (has `Display`), `IpcReceiveError { ConnectionNotFound, NoMessagesAvailable, MailboxNotAvailable }` (has `Display`).
- `IpcMessage { pub data: usize, pub connection_handle: IpcConnectionHandle }` (`Copy`).
- `IpcMessageFuture { message: Option<IpcMessage>, error: Option<IpcReceiveError> }` — implements the `Future` trait; constructors `new()`, `with_message()`, `with_error()`, plus `complete()`, `result()`.

**`system/src/future.rs`**:
- `FutureHandle = Handle`.
- `enum FutureResult { IpcMessage(Result<IpcMessage, IpcReceiveError>), Void }`.
- `trait Future: Send + Sync { fn is_completed(&self) -> bool; fn into_result(self: Box<Self>) -> FutureResult; fn as_any(&self) -> &dyn Any; fn as_any_mut(&mut self) -> &mut dyn Any; fn into_any(self: Box<Self>) -> Box<dyn Any + Send + Sync>; }`.

**`system/src/syscall_numbers.rs`** — `SyscallNum` enum, IPC arms: `IpcConnect=11, IpcDisconnect=12, IpcSend=13, IpcReceive=14, IpcBind=15, IpcReceiveFromClient=16, IpcSendToClient=17`. **These numeric values are ABI and must not change.**

**`kernel/src/ipc/`**:
- `mailbox.rs` → `Mailbox { queue: VecDeque<IpcMessage> }` with `new()`, `push_back()`, `is_empty()`, `pop_front()`.
- `mailbox_manager.rs` → `MailboxManager { mailboxes: GenerationalArena<Mailbox,256>, waiters: BTreeMap<MailboxHandle, Vec<FutureHandle>> }`. Methods:
  - `create() -> MailboxHandle`, `remove(handle)`.
  - `push_back(handle, msg)`: enqueues, then `notify_waiters`.
  - `pop_front_async(handle) -> FutureHandle`: if a message is queued → register an **immediately-completed** future (`IpcMessageFuture::with_message`); if empty → register a **pending** future and push its handle onto `waiters[handle]`; if the mailbox handle is gone → register an error future (`MailboxNotAvailable`).
  - `notify_waiters(handle)`: while messages remain, pop a future handle from `waiters[handle]`, `downcast_mut` the boxed future to `IpcMessageFuture`, `complete(msg)`, then `services().future_registry.borrow_mut().notify(fh)` (wakes the waiting task). **This is where the async completion is wired to the scheduler.**
  - `type MailboxHandle = Handle`.
- `ipc_manager.rs` → `IpcManager { bindings: GenerationalArena<IpcServerBinding,256>, mailbox_manager: MailboxManager, connections: GenerationalArena<IpcConnection,256>, registry: BTreeMap<String, IpcBindingHandle> }`. Inner types `IpcServerBinding { service: String, mailbox_handle: MailboxHandle }`, `IpcConnection { server_mailbox, client_mailbox }`. Methods (the use-cases):
  - `bind_service(name) -> Result<IpcBindingHandle, IpcBindingError>` (dup → `AlreadyBound`; creates a mailbox + binding + registry entry).
  - `connect(name) -> Result<IpcConnectionHandle, IpcConnectionError>` (finds binding, creates client mailbox + `IpcConnection`).
  - `disconnect(conn)` (removes connection + its client mailbox).
  - `send_to_server(msg) -> Result<(), IpcSendError>` / `send_to_client(msg)` (look up connection by `msg.connection_handle`, push into the right mailbox).
  - `receive_from_all_clients_async(binding) -> FutureHandle` / `receive_from_server_async(conn) -> FutureHandle` (delegate to `mailbox_manager.pop_front_async`; on missing handle → register `IpcMessageFuture::with_error(ConnectionNotFound)` **via `alloc::boxed::Box`**).

**`kernel/src/future.rs`** (the outbound async adapter):
- `TaskFuture`, `TaskCompletionFuture`, `TimeFuture` (other future kinds — do not disturb).
- `FutureRegistry { arena: GenerationalArena<Box<dyn Future + Send + Sync>,1024>, waiters: BTreeMap<FutureHandle, Vec<TaskHandle>> }`.
  - `register(Box<dyn Future>) -> Option<FutureHandle>`, `register_waiter(fh, task)`, `get(fh) -> Option<bool>` (completed?), `consume(fh) -> Result<Box<dyn Future>, _>`, `borrow_mut(fh)`, `replace(fh, future)`.
  - `notify(fh)`: removes `waiters[fh]` and calls **`services().scheduler.borrow_mut().wake_tasks(waiters)`** → this is the actual task wakeup.

**`kernel/src/kernel_cell.rs`** → `KernelCell<T>` = `UnsafeCell<T>` with `borrow()`/`borrow_mut()`/`replace()` (NOT a lock — a raw interior-mutability cell; `unsafe impl Sync`). Used as the field wrapper in services.

**`kernel/src/kernel_services.rs`** → `KernelServices { task_manager, future_registry, ipc_manager, timer_manager, scheduler, memory_manager }`, each a `KernelCell<T>` (memory is `&'static MemoryManager`). Global `static KERNEL_SERVICES: Once<KernelServices>`; `init()` and `services() -> &'static KernelServices`. **`init()` has a `#[cfg(not(test))]` and a `#[cfg(test)]` branch** — the test branch uses a `std::sync::OnceLock` so the same global works under host `cargo test`. **This dual path is how IPC tests run on the host today and must be preserved.**

**`kernel/src/syscall.rs`** (inbound driver) → the `Ipc*` arms of the dispatcher call `services().ipc_manager.borrow_mut()`. Two return styles cross the syscall boundary:
- `Result` values are **heap-boxed and returned as a raw pointer**: `Box::into_raw(Box::new(result)) as usize` (e.g. `IpcConnect`, `IpcBind`, `IpcSend`).
- `FutureHandle`s are **packed into one `usize`**: `.pack()` (e.g. `IpcReceive`, `IpcReceiveFromClient`).

**`usrlib/src/syscall.rs`** (user-space API, the mirror of the above) → `ipc_connect(&str)`, `ipc_disconnect`, `ipc_send`, `ipc_receive`, `ipc_bind`, `ipc_receive_from_client`, `ipc_send_to_client`. It `unpack()`s the `FutureHandle` and deboxes the `Result` pointers. **The syscall number + argument layout is the ABI; keep it stable.**

### 2.3 Reference user-space service (the model we are protecting)

`apps/random_gen_server/src/main.rs` (excluded from workspace; built separately) is the canonical **user-space service**:
```
_start → Syscall::ipc_bind("RANDOM")
loop {
  fh = Syscall::ipc_receive_from_client(binding);
  if let IpcMessage(Ok(msg)) = Syscall::wait_future(fh) {
     value = self.next() as usize;
     Syscall::ipc_send_to_client(msg.connection_handle, value);
  }
}
```
And the client side in `apps/shell/src/shell.rs::random()`:
```
conn = Syscall::ipc_connect("RANDOM");
Syscall::ipc_send(conn, 123456);
fh = Syscall::ipc_receive(conn);
msg = match Syscall::wait_future(fh) { IpcMessage(Ok(m)) => m, _ => none };
Syscall::ipc_disconnect(conn);
```
**Filesystem / networking will follow this exact client/server pattern.** If this still works after the refactor, the extension story is intact.

### 2.4 End-to-end data flow (must remain intact)

```
app (server)                     app (client)
  bind("S")                        connect("S")
  recv_from_client → fh            send(conn, v)
  wait_future(fh)                  recv → fh2
  send_to_client(v2)               wait_future(fh2)
  disconnect?                      disconnect(conn)

kernel: syscall → IpcManager → MailboxManager(Mailbox) ──(queue)──▶
  empty recv ⇒ FutureRegistry.register(pending IpcMessageFuture) + waiter list
  send       ⇒ push_back + notify_waiters ⇒ FutureRegistry.notify ⇒ Scheduler.wake_tasks
  blocked task ⇒ wait_future blocks; Scheduler re-runs it when woken
```

---

## 3. Target Architecture (the "to-be")

See `plans/ipc-architecture.md` for the diagram + rationale. Summary:

```
   adapters (kernel + syscall + usrlib)
   ┌───────────────────────────────────────────────────────────────────┐
   │ inbound:  syscall (driver)    usrlib (driver, user side)         │
   │ outbound: MailboxManager      KernelCompletionEngine (FutureReg-  │
   │          (mailbox port)        istry) + Scheduler (completion +  │
   │                                 task-wake + error ports)          │
   └───────────────┬───────────────────────────────────────────────────┘
                   │ depends only on
        ┌──────────▼───────────┐      ┌─────────────────────────────┐
        │ Application layer    │      │ Domain (pure value types)   │
        │ IpcBroker (use-cases:│◄────►│ IpcMessage, IpcConnection,  │
        │ bind/connect/send/   │      │ IpcBinding, IpcServerBinding,│
        │ receive/disconnect)  │      │ error enums                 │
        └──────────┬───────────┘      └─────────────────────────────┘
                   │ through ports (traits) in `system::ipc`
        ┌──────────▼───────────┐
        │ Ports (system crate) │  MessageQueuePort / CompletionEngine
        │                      │  TaskWaker / IpcErrorReporter
        └──────────────────────┘
```

**The three ports** (new traits in `system/src/ipc.rs`, defined by the broker, implemented by kernel adapters):

```rust
// OUTBOUND: mailbox queue storage (implemented by MailboxManager)
pub trait MessageQueuePort {
    fn create_queue(&mut self) -> MailboxHandle;          // MailboxHandle = Handle
    fn drop_queue(&mut self, handle: MailboxHandle);
    fn push(&mut self, handle: MailboxHandle, message: IpcMessage);
    fn pop(&mut self, handle: MailboxHandle) -> Option<IpcMessage>;
    fn has(&self, handle: MailboxHandle) -> bool;         // replaces borrow(is_ok)
}

// OUTBOUND: async completion (implemented by a kernel CompletionEngine over FutureRegistry)
pub trait CompletionEngine {
    fn register_message(&mut self, message: IpcMessage) -> FutureHandle;      // completed now
    fn register_pending_message(&mut self) -> FutureHandle;                    // pending
    fn register_error_message(&mut self, error: IpcReceiveError) -> FutureHandle;
    fn attach_waiter(&mut self, mailbox: MailboxHandle, future: FutureHandle);
    fn drain_waiters(&mut self, mailbox: MailboxHandle) -> Vec<FutureHandle>;  // remove + return
    fn complete(&mut self, future: FutureHandle, message: IpcMessage);
    fn wake(&mut self, future: FutureHandle);                                       // scheduler wake
}

// OUTBOUND: task wakeup (thin; implemented by Scheduler adapter)
pub trait TaskWaker {
    fn wake_tasks(&mut self, handles: Vec<TaskHandle>);
}

// OUTBOUND: reporting (implemented by kprint/panic)
pub trait IpcErrorReporter {
    fn report_send(&self, error: IpcSendError);
    fn report_bind(&self, error: IpcBindingError);
    fn report_connection(&self, error: IpcConnectionError);
}
```

> **Why ports live in `system`:** `system` already holds the IPC value types and `IpcMessageFuture` and is shared by `usrlib`/`arch`. Defining ports there keeps the broker 100% free of `crate::kernel_services` / `alloc::boxed::Box`, which is what makes host unit tests trivial. `TaskHandle` is referenced by `TaskWaker` (see open item **Q1** — how to keep `TaskHandle` off the trait without a circular dep).

**Invariants preserved through the whole refactor (non-negotiable):**
1. The **syscall ABI** (numbers + arg layout + the `Box::into_raw` / `pack()` return conventions) is unchanged → `usrlib` and existing apps keep working with **no edits**.
2. **Observable behavior** of `bind/connect/send/receive/disconnect` is identical, including the async semantics: empty receive returns a *pending* future that later completes (and wakes its task) when a message arrives; a missing handle yields an *error* future.
3. **Ownership semantics unchanged:** `bind` creates a server mailbox that lives until shutdown (current behavior — `disconnect` only removes the client mailbox; do not "fix" this here).
4. **No behavior fixes** smuggled in (e.g. do not add back-pressure for `ConnectionCongested`, do not change error variants). Pure reorganization.

---

## 4. Execution Plan (one commit per step; verify + commit before the next)

> **Commit-message convention (from AGENTS.md):** no prefix; e.g. `Add baseline IPC behavior tests`. Ask for confirmation on each commit message before committing.

### Step 0 — Safety net / baseline (commit 1)
**Goal:** pin current behavior with host-runnable tests so later steps can't silently regress it.
**Where:** new `#[cfg(test)]` in `kernel/src/ipc/ipc_manager.rs` (reuse the existing `crate::kernel_services::init as init_services` pattern used in `mailbox_manager.rs` tests).
**Tests to add** (these exercise the real `FutureRegistry`+`Scheduler`, which exist under the `#[cfg(test)]` init path):
- `bind_connect_roundtrip`: bind("S") → connect("S") → `send_to_server` → `receive_from_server_async` returns a future whose `result()` is `Ok(msg with the sent data)`.
- `bind_already_bound`: second `bind("S")` → `Err(AlreadyBound)`.
- `connect_unknown`: `connect("NOPE")` → `Err(ServerNotFound)`.
- `send_unknown_conn`: `send_to_server` with a bogus `IpcConnectionHandle` → `Err(ConnectionNotFound)`.
- `empty_receive_is_pending_then_completes`: connect, `receive_from_server_async` (no data) → future not completed; then `send_to_server`; assert the *same* future handle now `is_completed` and holds the message (assert via `FutureRegistry::get` / `consume`). This pins the async completion path end-to-end.
- `disconnect_then_send`: connect, disconnect, `send_to_server` → `Err(ConnectionNotFound)`.

**Do:** run `cargo test -p kernel` — all new + existing green.
**Commit:** `Add baseline IPC behavior tests`.
**Gate:** nothing else changes; this commit must be green on its own.

### Step 1 — Introduce the ports in `system` (commit 2)
**Goal:** add the trait definitions (no wiring yet) so the broker can be written against them.
**Where:** `system/src/ipc.rs` (add the traits from §3). Possibly a tiny `pub use`/type alias if `TaskHandle` is needed (see **Q1**).
**Do:** ensure `system` still compiles for host + the no_std build; no consumer uses the traits yet.
**Risk:** low (additive). **Watch:** keep `system` `no_std`-clean; `TaskHandle` origin (**Q1**).
**Commit:** `Add IPC ports to system crate`.

### Step 2 — Implement `MessageQueuePort` in `MailboxManager` (commit 3)
**Goal:** make `MailboxManager` satisfy `MessageQueuePort` with thin methods that delegate to existing logic.
**Where:** `kernel/src/ipc/mailbox_manager.rs`.
**Do:**
- `impl MessageQueuePort for MailboxManager { ... }` — `create_queue`→`create`, `drop_queue`→`remove`, `push`→`push_back`, `pop`→ a new `pop_one` (single `pop_front`), `has`→ `self.mailboxes.borrow(handle).is_ok()`.
- Refactor the internal `push_back`/`pop_front_async`/`notify_waiters` so they **route through the port methods** (single source of truth), but keep the same async side-effects for now (they still call `services().future_registry` directly — that moves in Step 4).
- Keep existing public methods working (they can become thin wrappers over the port) so `IpcManager` still compiles unchanged in this step.
**Verify:** `cargo test -p kernel` (Step-0 tests still green).
**Commit:** `Make MailboxManager a MessageQueuePort`.

### Step 3 — Introduce the kernel `CompletionEngine` (commit 4)
**Goal:** move the `FutureRegistry`-dependent async logic behind a kernel-side `CompletionEngine` type that implements the `CompletionEngine` port. This is the type that will later be *injected* into the broker.
**Where:** new file `kernel/src/ipc/completion_engine.rs` (module added to `ipc/mod.rs`); it wraps/reaches `services().future_registry` (+ `scheduler`) exactly as today.
**Do:**
- `pub struct CompletionEngine;` (stateless over the global registry) implementing:
  - `register_message` → `services().future_registry.borrow_mut().register(Box::new(IpcMessageFuture::with_message(m)))`
  - `register_pending_message` → same with `IpcMessageFuture::new()`
  - `register_error_message` → same with `with_error(e)`
  - `attach_waiter(mailbox, future)` → **needs to reach the `MailboxManager.waiters` map** (see **Q2** — who owns the waiter list). Preferred: keep the waiter list in `MailboxManager` and expose it via the `MessageQueuePort` (`attach_waiter`/`drain_waiters` on the *queue* port, or a combined `AsyncQueuePort`). Resolve ownership here.
  - `drain_waiters` → `waiters.remove(&mailbox).unwrap_or_default()`
  - `complete` → `downcast_mut::<IpcMessageFuture>()` + `complete(m)` (as in today's `notify_waiters`)
  - `wake` → `services().future_registry.borrow_mut().notify(fh)`
- `impl TaskWaker for Scheduler` (or a tiny adapter) → `wake_tasks` (thin, already exists).
- **Do NOT change behavior** — this type just *contains* today's logic.
**Verify:** `cargo test -p kernel`.
**Commit:** `Add kernel CompletionEngine for async IPC completion`.

### Step 4 — Refactor `MailboxManager.push/pop/notify` to use the engine (commit 5)
**Goal:** `MailboxManager` no longer calls `services().future_registry` directly for completion; it calls the injected/available `CompletionEngine`.
**Where:** `kernel/src/ipc/mailbox_manager.rs`.
**Do:**
- `push_back` → `push` (port) + `drain_waiters` + loop `pop` → `engine.complete` → `engine.wake`.
- `pop_front_async` → `pop`; Some → `engine.register_message`; None → `engine.register_pending_message` + `engine.attach_waiter`; missing handle → `engine.register_error_message(MailboxNotAvailable)`.
- The `engine` here may be a local `CompletionEngine` value (stateless) — no `&mut` capture problems since it just forwards to `services()`.
**Verify:** Step-0 tests green (esp. `empty_receive_is_pending_then_completes`).
**Commit:** `Route MailboxManager completion through CompletionEngine`.

### Step 5 — Rewrite `IpcManager` as `IpcBroker` depending only on ports (commit 6)
**Goal:** the core hexagon win. `IpcManager` (rename to `IpcBroker` or keep the name — see **Q3**) holds ports as fields and never references `services()` or `alloc::boxed::Box` for completion.
**Where:** `kernel/src/ipc/ipc_manager.rs`.
**Do:**
- New fields: `queue: MailboxManager` (implements `MessageQueuePort`), `completion: CompletionEngine`, (and the reporter if wired). Keep `bindings`, `connections`, `registry`.
- `bind_service` → use `queue.create_queue()`; on `AlreadyBound` optionally `reporter.report_bind` (keep returning the `Err`).
- `connect` → `queue.create_queue()` for the client mailbox; on not found `reporter.report_connection`.
- `send_to_server/client` → `queue.push(...)`; on missing conn `reporter.report_send`.
- `receive_*_async` → delegate to `queue` + `completion` (the error-future path now uses `completion.register_error_message`, **removing the `alloc::boxed::Box` from this file**).
- `disconnect` → `queue.drop_queue(client_mailbox)`.
**Verify:** `cargo test -p kernel`; then **build the full x86_64 image** (see §5) and run the `random` shell flow if a harness exists.
**Commit:** `Refactor IpcManager to depend only on IPC ports`.

### Step 6 — Wire the ports in `KernelServices::init` + syscall (commit 7)
**Goal:** make the injected ports the *real* singletons, keeping the syscall layer as the only inbound driver.
**Where:** `kernel/src/kernel_services.rs`, `kernel/src/syscall.rs`.
**Do:**
- Construct `IpcManager` with the concrete `MailboxManager` + `CompletionEngine` (and reporter). The `services()` global is now only used *inside* the adapters, never in the broker.
- Keep syscall `Ipc*` arms calling the same broker methods; **ABI unchanged**.
- (Optional, low value) add a `IpcBroker::new_default()` that builds the concrete adapters, so tests and `init` share one construction path.
**Verify:** full build + `cargo test -p kernel` + (if available) run `random_gen_server` + shell `random`.
**Commit:** `Wire IPC ports into KernelServices and syscall`.

### Step 7 — Host unit tests for the broker against fake ports (commit 8)
**Goal:** prove the decoupling — test `IpcBroker` on the host with **fake in-memory ports**, no `services()` boot at all.
**Where:** `#[cfg(test)]` in `ipc/ipc_manager.rs` (or a new `broker_tests` module).
**Do:**
- `FakeQueue` (HashMap<Handle, VecDeque<IpcMessage>>) impl `MessageQueuePort`.
- `FakeCompletion` (records registered futures + waiters, can manually "complete") impl `CompletionEngine`.
- `FakeWaker` / `FakeReporter` (collect calls).
- Tests: `bind/connect/disconnect` state machine; `send` reaches the right queue; `receive` on empty → pending + waiter attached; manual complete → wake recorded; error paths call the reporter. These run **without `init_services()`**.
**Verify:** `cargo test -p kernel` (both the real-adapter tests from Step 0 and the fake-port tests pass).
**Commit:** `Add host unit tests for IPC broker with fake ports`.

### Step 8 — Documentation + close-out (commit 9)
**Goal:** make the architecture durable for the next agent and for adding services.
**Do:**
- Update `AGENTS.md`: fix the stale "IPC = mailbox manager" line to describe the broker + ports + user-space-service model; note the port traits live in `system::ipc`. (AGENTS.md is currently stale in several places — see §7.)
- In `plans/ipc-architecture.md`, add a short **"How to add a new service (filesystem/networking)"** section: create a user-space app that `ipc_bind("FS")` and loops on `ipc_receive_from_client`; clients `ipc_connect("FS")`/`ipc_send`. Reference `random_gen_server` as the template.
- Mark this plan file `Status: Done` (or `In progress`) and record any deferred items.
**Commit:** `Document IPC broker architecture and service-extension guide`.

---

## 5. Build & test commands

```bash
# host unit tests (fast; this is where Steps 0–7 are verified)
cargo test -p kernel

# build the x86_64 kernel (no_std) — must stay green after every kernel change
cargo build -p rosx --release            # bin name for arch/x86_64

# x86_32 must still compile (shared kernel/system); do NOT edit it, just check
cargo build -p rosx-x86

# bootable image (from CI: arch/x86_64/run.sh is the runner; build-image via the runner workspace)
cargo run --manifest-path arch/x86_64-runner/Cargo.toml -- <kernel-binary> x86_64 --no-run

# excluded user-space service (built separately, template for new services)
cargo run --manifest-path apps/random_gen_server/Cargo.toml --   # if it has its own target setup
```

**Definition of done for the whole effort:** Step-0 tests green, fake-port tests green, `cargo build -p rosx` and `-p rosx-x86` green, and the `random` client/server flow still works (or the closest runnable equivalent). No `crate::kernel_services` reference remains in the broker file; no `alloc::boxed::Box` remains in the broker for completion.

---

## 6. Risks & rollback

- **R1 — Async completion timing.** The subtlety: `receive` when empty must return a *pending* future whose handle the *caller* later blocks on via `wait_future`, and a later `send` must complete *that same* future and wake its task. If Step 4/5 changes which future handle is returned, the blocked task will never wake. **Mitigation:** the Step-0 test `empty_receive_is_pending_then_completes` asserts the *same handle* transitions; do not change the handle-identity contract.
- **R2 — `services()` re-introduction.** The broker must not call `services()` directly. Grep the broker file after Step 5: `grep -n "services()\|alloc::boxed::Box" kernel/src/ipc/ipc_manager.rs` should be empty.
- **R3 — ABI drift.** Any change to `syscall.rs` `Ipc*` arg layout or `pack()`/`Box::into_raw` convention breaks `usrlib` + apps silently. **Rule:** syscall layer stays byte-identical in this effort.
- **R4 — Waiter-list ownership.** `MailboxManager.waiters` and `FutureRegistry.waiters` are two different waiter maps (mailbox→future vs future→task). Splitting them carelessly drops a wake. **Mitigation:** Step 3/4 must keep `attach`/`drain`/`wake` matching today's `notify_waiters` exactly; the Step-0 test covers it.
- **R5 — `system` crate boundary.** `system` is shared with `usrlib`/`arch` (user side). Ports are fine there, but don't accidentally make user-space depend on kernel-only types (**Q1** on `TaskHandle`).
- **Rollback:** each step is its own commit and behavior-preserving, so `git revert <sha>` (or checkout the parent) restores the prior working state. Never let a step land that isn't independently green.

---

## 7. Housekeeping / correctness notes (flag, don't fix in this plan)

These are real issues seen while mapping the code; out of scope for the port refactor but recorded so we don't lose them:

1. **`AGENTS.md` is stale** (says "last updated 2025-10-12"; describes x86_32 as the only extra platform, IPC as "a mailbox manager", memory as pluggable). Memory is *not* pluggable today (hard `global_allocator` static `MEMORY_MANAGER` → `FreeListAllocator`; `bitmap_chunk_allocator` exists but isn't selectable). Update in Step 8.
2. **Kernel has two `.S` names:** files on disk are `process_initialization.S` + `syscall.S` + `context_switching.S` (embedded via `global_asm!`); AGENTS/README refer to "process_initialization.S" vs "process_initialization.S" inconsistently. Cosmetic.
3. **`IpcManager::receive_*_async` error path uses `alloc::boxed::Box`** — Step 5 removes it (a real improvement, not a behavior change).
4. **`ConnectionCongested` is never produced** and there's no back-pressure — future work, not here.
5. **`IpcSendError`/`IpcReceiveError` `Display`** — `IpcReceiveError` has `Display`; confirm all error enums that cross to user-space have it (the shell prints some). Keep stable.

---

## 8. Open items to confirm before/at execution (answer, then proceed)

- **Q1 — `TaskHandle` on the `TaskWaker` port.** ✅ **Resolved:** `TaskHandle` *is* `collections::generational_arena::Handle` (`kernel/src/task.rs:10` → `pub(crate) type TaskHandle = Handle;`). So alias it in `system` (e.g. `pub type TaskHandle = Handle;`) and keep **all four** ports — including `TaskWaker` — in the pure `system` crate. No circular dep. No open question remains; proceed with this in Step 1.
- **Q2 — Who owns the mailbox→future waiter list?** Today it's `MailboxManager.waiters`. Preferred: keep it on the *queue* side (the `MessageQueuePort`), so `attach_waiter`/`drain_waiters` live there and `CompletionEngine` only does registry `complete`/`wake`. Confirm before Step 3.
- **Q3 — Rename `IpcManager` → `IpcBroker`?** Renaming improves DDD clarity but touches `kernel_services.rs` + `syscall.rs` references. **Lean: rename** (small, contained). Confirm.
- **Q4 — Do you want a runnable "echo" service added in Step 8** as a second template (alongside `random_gen_server`) to prove multi-service coexistence? Optional; **lean no** unless you want it.

---

## 9. Out of scope (explicit)

- Message payload abstraction / structured messages (kept `usize`).
- Any change to the syscall ABI, `usrlib` IPC wrappers, or existing apps.
- Back-pressure / congestion handling, message persistence, priorities, QoS.
- x86_32-specific work (it only must keep compiling).
- Scheduler, memory-manager, or other-subsystem refactors.
- Behavior bug-fixes (this is pure reorganization).

---

## 10. How an agent picks this up (checklist)

1. Read this file + `plans/ipc-architecture.md`. Re-verify the As-Is snapshot in §2 against the current tree (line numbers drift).
2. `git status` + `git log --oneline -15` to see where you are.
3. Review the open items §8 (Q2–Q4; Q1 is resolved) — they gate Steps 3 and 5.
4. Run `cargo test -p kernel` to confirm the baseline is green **before** touching anything.
5. Execute steps in order; for each: make the change → `cargo test -p kernel` → (for Steps 5–6) `cargo build -p rosx` and `-p rosx-x86` → propose a commit message → commit. **One concept per commit; stop and confirm between steps.**
6. Keep the §3 invariants + §6 risks in view; when in doubt, the Step-0 tests are the arbiter.
7. Finish with Step 8 (docs) and mark the plan `Done`.
