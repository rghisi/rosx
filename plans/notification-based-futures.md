# Plan: Replace Future Polling with Notification-Based Waking

> **Created:** 2026-06-06
> **Status:** Awaiting approval

---

## Current State — How Futures Work Now

The `Future` trait uses a **polling** model. Every scheduler loop iteration, the scheduler checks every blocked future:

```rust
// system/src/future.rs
pub trait Future: Send + Sync {
    fn is_completed(&self) -> bool;
    fn as_any(&self) -> &dyn Any;
}
```

```rust
// kernel/src/scheduler/scheduler.rs — current run() loop
pub fn run(&mut self) {
    loop {
        self.process_hardware_interrupts();
        self.poll_futures();  // ← iterates ALL blocked futures every iteration
        self.run_next_task();
    }
}
```

### All Future Implementations (4 total)

| Future | Location | What it polls |
|---|---|---|
| `TimeFuture` | `kernel/src/future.rs` | System time vs completion timestamp |
| `TaskCompletionFuture` | `kernel/src/future.rs` | Task state == `Terminated` |
| `KeyboardFuture` | `kernel/src/keyboard.rs` | Keyboard buffer not empty |
| `IpcReplyFuture` | `system/src/ipc.rs` | Reply value is `Some` |

### The Problem with Polling

- **Latency:** A task can only wake up on the next scheduler iteration after an event occurs
- **Wasted cycles:** Every loop iterates ALL blocked futures, even if none changed
- **Orphaned timer module:** `scheduler/timer.rs` exists but is never wired in

---

## Working Rules (MANDATORY)

For every step below, follow the **RED → GREEN → REFACTOR** TDD cycle:

1. **RED** — Write or update a test that fails because the new behavior doesn't exist yet
2. **GREEN** — Implement just enough code to make the test pass
3. **REFACTOR** — Clean up (remove dead code, rename, simplify) — only if tests still pass
4. **Commit** after each step completes with a descriptive message (no prefix)
5. **Never skip ahead** — each step must compile and all existing tests must pass before starting the next

---

## Proposed Architecture

```
┌──────────────────────────────────────────────────┐
│              FutureRegistry                       │
│                                                   │
│  handles: GenerationalArena<Box<dyn Future>>      │
│  waiters: HashMap<FutureHandle, Vec<TaskHandle>>  │
│                                                   │
│  notify(handle) → wake all waiters                │
│  register(future, waiter_task) → handle           │
└──────────────────────┬───────────────────────────┘
                       │
          ┌────────────┼──────────┬──────────┐
          ▼            ▼          ▼          ▼
     TimeFuture   TaskComp    Keyboard  IpcReply
     (timer tick) (task mgr) (interrupt)(IPC sender)
```

---

## Step-by-Step Plan

### Step 1: Add `notify()` to `FutureRegistry` and Waiter Tracking - [DONE]

**Files:** `system/src/future.rs`, `kernel/src/future.rs`

Add a `waiters: HashMap<FutureHandle, Vec<TaskHandle>>` field to `FutureRegistry`. When a task waits on a future during registration, the registry records which task is waiting. Add `notify(handle)` method that wakes all waiters and removes them from the map.

**RED**
Write a test in `kernel/src/future.rs` that:
- Registers a future with a waiter task handle
- Calls `registry.notify(handle)`
- Asserts the waiter task was "woken" (track via a simple mechanism — e.g., a `Vec<TaskHandle>` of woken tasks)

**GREEN**
- Add `waiters: HashMap<FutureHandle, Vec<TaskHandle>>` to `FutureRegistry`
- Add `fn notify(&mut self, handle: FutureHandle) -> Vec<TaskHandle>` — wakes all waiters, removes from map, returns woken handles
- Modify `register()` to accept an optional `waiter_task: Option<TaskHandle>` parameter
- When registering with a waiter, store it in the map

**REFACTOR**
- N/A (new code)
- Verify existing tests still pass

---

### Step 2: Wire Up TaskCompletionFuture Notifications - [DONE]

**Files:** `kernel/src/task_manager.rs`, `kernel/src/kernel.rs`

When a task transitions to `Terminated`, notify any tasks waiting on its completion future.

**RED**
Write a test that:
- Creates a task with a completion future
- Registers the task as waiting on that future (via Step 1's new API)
- Sets the task state to `Terminated`
- Asserts `registry.notify()` would return the waiter task

**GREEN**
- In `TaskManager::set_state()`, when transitioning to `Terminated`, look up the task's `completion_future` handle and call `services().future_registry.borrow_mut().notify(completion_handle)`
- This automatically wakes any waiting tasks — no scheduler polling needed

**REFACTOR**
- Verify existing tests still pass (especially the scheduler termination tests)

---

### Step 3: Wire Up KeyboardFuture Notifications

**Files:** `kernel/src/keyboard.rs`

When a key arrives and the buffer transitions from empty → non-empty, notify any task waiting for keyboard input.

**RED**
Write a test that:
- Creates a `KeyboardFuture` and registers it with a waiter
- Pushes a key to the buffer (empty → non-empty transition)
- Asserts the future's notify path fires for that waiter

**GREEN**
- Add a static `KernelCell<Option<FutureHandle>>` to track which future is waiting for keyboard input
- In `push_key()`: if this is the first key (buffer was empty), look up the registered future handle and call `services().future_registry.borrow_mut().notify(handle)`
- The syscall path (`ReadChar` in `syscall.rs`) will register the future with its waiter via Step 1's API

**REFACTOR**
- N/A

---

### Step 4: Wire Up IpcReplyFuture Notifications

**Files:** `kernel/src/ipc/ipc_manager.rs`

When an IPC reply is sent, notify any task waiting for that reply.

**RED**
Write a test that:
- Registers an IPC future with a waiter
- Calls `reply()` with a reply message
- Asserts the waiter is notified

**GREEN**
- In `IpcManager::reply()`, after replacing the future with one containing the reply, call `services().future_registry.borrow_mut().notify(reply_future_handle)` to wake the waiting task

**REFACTOR**
- N/A

---

### Step 5: Integrate Timer Module — Notification-Based Sleep

**Files:** `kernel/src/scheduler/timer.rs`, `kernel/src/kernel.rs`, `kernel/src/scheduler/scheduler.rs`

Use the existing (but unused) timer module to notify sleep futures at their deadline.

**RED**
Write a test that:
- Creates a timer with a future handle at deadline T=50
- Calls `timer.pop_expired_and_notify(50, registry)`
- Asserts the future handle was notified (waiters woken)

**GREEN**
- Change `Timer::next` to store `(FutureHandle, TaskHandle)` pairs instead of just `FutureHandle`
- Add `pub fn pop_expired_and_notify(&mut self, now: u64, registry: &mut FutureRegistry)` — pops expired entries and calls `registry.notify(handle)` for each
- Update `add_sleep()` to accept `(FutureHandle, TaskHandle)`
- In `kernel().wait_future()`: when registering a TimeFuture for sleep, also register it in the timer with the waiter task

**REFACTOR**
- N/A

---

### Step 6: Remove Polling Infrastructure — The Grand Cleanup

**Files:** `kernel/src/scheduler/scheduler.rs`, `kernel/src/future.rs`

Remove all polling-related code now that notifications handle waking tasks.

**RED**
Write an integration test that verifies the scheduler loop works correctly without `poll_futures()`:
- A task blocks on a sleep future → timer tick notifies → task wakes
- A task blocks waiting for another task to terminate → that task terminates → notification wakes it

**GREEN**
- Remove `blocked_tasks: VecDeque<TaskFuture>` from `Scheduler`
- Remove `fn push_blocked()` from `Scheduler`
- Remove `fn poll_futures()` from `Scheduler`
- Remove `TaskFuture` struct from `kernel/src/future.rs`
- Remove `cleanup_completion_future()` from `Scheduler` (no longer needed — termination notifies directly)
- Update `run()` loop:
  ```rust
  pub fn run(&mut self) {
      loop {
          self.process_hardware_interrupts();
          // Timer-based notifications happen via kernel().timer.tick()
          // or directly through future_registry.notify()
          self.run_next_task();
      }
  }
  ```

**REFACTOR**
- Remove `push_blocked` test from scheduler tests
- Remove `waited_on_completion_future_is_preserved_when_task_terminated` test (no longer relevant)
- Clean up any unused imports
- Verify all remaining tests pass

---

### Step 7: Update `wait_future()` — Single Registration Path - [DONE]

**Files:** `kernel/src/kernel.rs`

Modify `kernel().wait_future()` to register the task with `FutureRegistry` so notifications work. This is the single registration point where a task blocks on a future.

**RED**
Test that `wait_future()` registers the task with `FutureRegistry` using the new API from Step 1.

**GREEN**
- Modify `kernel().wait_future()` to:
  1. Block the current task (existing)
  2. Register the future **with** the waiter task handle via `register(future, Some(task_handle))`
  3. Switch to scheduler (existing)
  4. Consume and return the future (existing)

**REFACTOR**
- The old path of `push_blocked()` is now obsolete — this is the single registration point

---

## Dependency Graph

```
Step 1 (FutureRegistry notify + waiters)
  ├── Step 7 (Update wait_future — depends on Step 1)
  ├── Step 2 (TaskCompletionFuture termination notification — depends on Step 1)
  ├── Step 3 (KeyboardFuture empty→non-empty notification — depends on Step 1)
  ├── Step 4 (IpcReplyFuture reply notification — depends on Step 1)
  └── Step 5 (Timer module integration for sleep — depends on Step 1)

Step 6 (Remove polling — depends on ALL of 2, 3, 4, 5, 7)
```

**Recommended execution order:** 1 → 7 → 2 → 3 → 4 → 5 → 6

Steps 2, 3, 4 are independent of each other and can be done in any order after step 1.

---

## Files Affected

| File | Changes |
|---|---|
| `system/src/future.rs` | No changes (trait stays the same) |
| `kernel/src/future.rs` | Add waiter tracking + `notify()` to `FutureRegistry`; remove `TaskFuture` struct |
| `kernel/src/kernel.rs` | Update `wait_future()` to register waiter; wire timer registration for sleep |
| `kernel/src/task_manager.rs` | Call `notify()` on task termination |
| `kernel/src/keyboard.rs` | Track waiting future handle; notify on empty→non-empty transition |
| `kernel/src/ipc/ipc_manager.rs` | Notify on IPC reply |
| `kernel/src/scheduler/timer.rs` | Change to store `(FutureHandle, TaskHandle)` pairs; add `notify_expired()` |
| `kernel/src/scheduler/scheduler.rs` | Remove `blocked_tasks`, `push_blocked()`, `poll_futures()`; simplify `run()` loop |

---

## Design Decisions

1. **Keep `is_completed()` on the trait** — Used by the `IsFutureCompleted` syscall; can be refactored away later
2. **Use timer module** — It exists but is unused; fits naturally with notification-based sleep
3. **Keyboard: empty→non-empty transition only** — Prevents spurious wakeups when multiple keys arrive in quick succession
4. **Interrupt-safe notifications** — `FutureRegistry` uses `KernelCell` (interior mutability), safe to call from interrupt context for now; can be improved later
5. **Incremental steps** — Each step is independently testable and compilable
