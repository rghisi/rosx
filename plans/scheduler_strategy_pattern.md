# Scheduler Refactoring: Strategy Pattern

**Date:** 2026-06-06
**Goal:** Eliminate duplication between `FifoScheduler` and `MlfqScheduler` using the Strategy pattern, where a single `Scheduler` engine struct delegates scheduling decisions to pluggable algorithms.

---

## Current State (Before)

### Files involved
| File | Lines | Role |
|---|---|---|
| `kernel/src/scheduler/mod.rs` | ~30 | `Scheduler` trait, factory functions |
| `kernel/src/scheduler/fifo_scheduler.rs` | ~180 | FIFO scheduler struct + impls + tests |
| `kernel/src/scheduler/mlfq_scheduler.rs` | ~240 | MLFQ scheduler struct + impls + tests |

### Duplicated code
- **Byte-for-byte identical** (`process_hardware_interrupts`, `cleanup_completion_future`) — ~35 lines × 2
- **Nearly identical patterns** (`push_task`, `push_blocked`, `poll_futures`/`pool_futures`) — shared logic repeated with minor queue-name differences
- **Identical idle task handling** in both schedulers' `run()` methods

### Issues to fix along the way
- FIFO has a typo: method named `pool_futures` instead of `poll_futures`

---

## Target Architecture (After)

```
┌─────────────────────────────────┐
│         Scheduler               │  ← Concrete struct, no trait
│                                 │
│  Shared state:                  │
│    • blocked_tasks              │
│    • hw_interrupt_queue         │
│    • idle_task                  │
│                                 │
│  Shared methods:                │
│    • run()                      │
│    • process_hardware_interrupts│
│    • poll_futures               │
│    • cleanup_completion_future  │
│    • set_idle_task              │
│    • push_task (forward)        │
│    • push_blocked (forward)     │
│    • push_hardware_interrupt    │
│    • should_preempt (forward)   │
│                                 │
│  ┌───────────────────────────┐  │
│  │  Box<dyn SchedulingAlgo>  │  │  ← Pluggable strategy
│  └───────────────────────────┘  │
└─────────────────────────────────┘

        ┌───────────────────────┐
        │ SchedulingAlgorithm   │  ← New trait
        │                       │
        │ fn pick_next()         │ → Option<TaskHandle>
        │ fn requeue_after_run() │ → ()
        │ fn push_ready()        │ → ()
        │ fn should_preempt()    │ → bool
        └───────────────────────┘
               ↙          ↘
       FifoStrategy    MlfqStrategy
```

---

## Execution Plan

### Step 0: Create the new file structure

**New files:**
- `kernel/src/scheduler/algorithm.rs` — New `SchedulingAlgorithm` trait definition + implementations will go here (or inline in their own files)
- `kernel/src/scheduler/fifo_strategy.rs` — `FifoStrategy` implementation
- `kernel/src/scheduler/mlfq_strategy.rs` — `MlfqStrategy` implementation

**Modified files:**
- `kernel/src/scheduler/mod.rs` — Replace the trait with new structure, update module declarations
- `kernel/src/scheduler/fifo_scheduler.rs` → **deleted**, content moves to `fifo_strategy.rs` + test adjustments
- `kernel/src/scheduler/mlfq_scheduler.rs` → **deleted**, content moves to `mlfq_strategy.rs` + test adjustments

**Approach:** Create new files first, then delete old ones (reduces merge conflicts and keeps git history clean).

---

### Step 1: Define the `SchedulingAlgorithm` trait (`algorithm.rs`)

Create `kernel/src/scheduler/algorithm.rs`:

```rust
use crate::task::{TaskHandle, TaskState};

pub trait SchedulingAlgorithm {
    /// Pick the next task to run. Returns None if no user tasks are ready.
    fn pick_next(&mut self) -> Option<TaskHandle>;

    /// Called after a task returns from switch_to_task — decide where it goes.
    /// `state` is the state the task was in when it returned (Running, Created, Ready, etc.).
    fn requeue_after_run(&mut self, handle: TaskHandle, state: TaskState);

    /// Push a newly-ready task into this scheduler's ready queue(s).
    fn push_ready(&mut self, handle: TaskHandle);

    /// Called during timer ticks to decide if the current task should be preempted.
    fn should_preempt(&mut self) -> bool;
}
```

**Key decisions:**
- `pick_next` returns `Option<TaskHandle>` — `None` means "no user tasks ready," and the engine falls back to idle
- `requeue_after_run` receives the task state as a parameter instead of reading it from services, keeping the strategy self-contained
- No accessor methods for internal queues — strategies encapsulate their own data structures. The engine only interacts through these four methods

---

### Step 2: Implement `FifoStrategy` (`fifo_strategy.rs`)

Create `kernel/src/scheduler/fifo_strategy.rs`:

```rust
use alloc::collections::VecDeque;
use crate::task::{TaskHandle, TaskState};
use super::algorithm::SchedulingAlgorithm;

pub struct FifoStrategy {
    ready_queue: VecDeque<TaskHandle>,
}

impl Default for FifoStrategy {
    fn default() -> Self {
        Self::new()
    }
}

impl FifoStrategy {
    pub fn new() -> Self {
        FifoStrategy {
            ready_queue: VecDeque::with_capacity(5),
        }
    }
}

impl SchedulingAlgorithm for FifoStrategy {
    fn pick_next(&mut self) -> Option<TaskHandle> {
        self.ready_queue.pop_front()
    }

    fn requeue_after_run(&mut self, handle: TaskHandle, _state: TaskState) {
        // FIFO always pushes back to the end of the queue regardless of state
        self.ready_queue.push_back(handle);
    }

    fn push_ready(&mut self, handle: TaskHandle) {
        self.ready_queue.push_back(handle);
    }

    fn should_preempt(&mut self) -> bool {
        true  // FIFO always preempts
    }
}

// Tests from fifo_scheduler.rs move here. Adjust any that called private methods on FifoScheduler.
```

**Changes from original `FifoScheduler`:**
- No `blocked_tasks`, `hw_interrupt_queue`, or `idle_task` — those stay in the engine
- No `process_hardware_interrupts`, `poll_futures`, `cleanup_completion_future` — those are in the engine now
- No `run()` method — that's on the engine struct
- The idle task special-case from `FifoScheduler::run_user_process()` (`if returned_task_handle != self.idle_task.unwrap()`) disappears entirely — requeueing is now unconditional for non-idle tasks, handled by the engine

---

### Step 3: Implement `MlfqStrategy` (`mlfq_strategy.rs`)

Create `kernel/src/scheduler/mlfq_strategy.rs`:

```rust
use alloc::collections::VecDeque;
use crate::task::{TaskHandle, TaskState};
use super::algorithm::SchedulingAlgorithm;

const NUM_QUEUES: usize = 3;
const QUANTA: [usize; NUM_QUEUES] = [2, 5, 10];

pub struct MlfqStrategy {
    queues: [VecDeque<TaskHandle>; NUM_QUEUES],
    remaining_quantum: usize,
}

impl Default for MlfqStrategy {
    fn default() -> Self {
        Self::new()
    }
}

impl MlfqStrategy {
    pub fn new() -> Self {
        MlfqStrategy {
            queues: [VecDeque::new(), VecDeque::new(), VecDeque::new()],
            remaining_quantum: 0usize,
        }
    }

    /// Determine next priority based on yield reason and current level.
    fn next_priority(current: usize, yield_reason: Option<YieldReason>) -> usize {
        match yield_reason {
            None => 0,
            Some(YieldReason::Voluntary) => current,
            Some(YieldReason::Preempted) => (current + 1).min(NUM_QUEUES - 1),
        }
    }

    fn reset_quantum(&mut self, priority: usize) {
        self.remaining_quantum = QUANTA[priority];
    }
}

impl SchedulingAlgorithm for MlfqStrategy {
    fn pick_next(&mut self) -> Option<TaskHandle> {
        for queue in self.queues.iter_mut() {
            if let Some(handle) = queue.pop_front() {
                return Some(handle);
            }
        }
        None
    }

    fn requeue_after_run(&mut self, handle: TaskHandle, state: TaskState) {
        // The engine passes the yield reason via a separate mechanism (see step 4).
        // For now, we need access to the yield reason — it can be stored on the strategy
        // during run(), or passed as a parameter. See design decision below.
        
        // APPROACH: Store current priority and yield reason in the engine's call.
        // The requeue_after_run will receive enough info to decide queue placement.
        self.queues[0].push_back(handle);  // Placeholder — see Step 4 for full logic
    }

    fn push_ready(&mut self, handle: TaskHandle) {
        self.queues[0].push_back(handle);
    }

    fn should_preempt(&mut self) -> bool {
        self.remaining_quantum = self.remaining_quantum.saturating_sub(1);
        self.remaining_quantum == 0
    }
}

// Tests from mlfq_scheduler.rs move here.
```

**⚠️ Design decision needed for `requeue_after_run`:**

The MLFQ strategy needs the **yield reason** and the task's **current queue index** to decide where to requeue it. Currently, this logic lives inside `MlfqScheduler::run_next_task()`, which has direct access to both. In the new design:
- The engine knows the yield reason (it reads it from `services().task_manager`)
- But the strategy doesn't know which queue index the task was at

**Two options:**

| Option | How | Pros | Cons |
|---|---|---|---|
| **A. Pass priority as parameter** | Change signature to `fn requeue_after_run(&mut self, handle: TaskHandle, state: TaskState, current_priority: usize)` | Simple, explicit, no extra storage in strategy | Requires engine to track and pass priority |
| **B. Strategy tracks yield reason internally** | Engine calls a method like `set_yield_reason(handle, reason)` before `requeue_after_run` | Keeps signature minimal | Adds another method, slightly more coupling |

**Recommendation: Option A.** Pass the current queue index (or priority level) as an additional parameter to `requeue_after_run`. This is explicit and keeps the strategy stateless about external context. The engine already tracks which task came from which path through `pick_next` (MLFQ can return the priority alongside the handle).

**Revised trait method:**
```rust
fn requeue_after_run(&mut self, handle: TaskHandle, current_priority: usize);
```

And `pick_next` for MLFQ returns just the handle — the engine tracks priority separately. Actually, let's think about this more carefully...

---

### Step 4: Reconcile the run loop design (important)

The engine's `run()` method needs to orchestrate everything. Here's how I see it working with priority tracking:

```rust
pub struct Scheduler {
    algorithm: Box<dyn SchedulingAlgorithm>,
    blocked_tasks: VecDeque<TaskFuture>,
    hw_interrupt_queue: VecDeque<HardwareInterrupt>,
    idle_task: Option<TaskHandle>,
}

impl Scheduler {
    pub fn new(algorithm: impl SchedulingAlgorithm + 'static) -> Self {
        Scheduler {
            algorithm: Box::new(algorithm),
            blocked_tasks: VecDeque::new(),
            hw_interrupt_queue: VecDeque::new(),
            idle_task: None,
        }
    }

    pub fn run(&mut self) {
        loop {
            self.process_hardware_interrupts();
            self.poll_futures();

            // Pick next task — for MLFQ this needs to return the priority too
            let (next_handle, current_priority) = match self.algorithm.pick_next() {
                Some(handle) => (handle, 0),  // FIFO doesn't track priority
                None => {
                    let idle = self.idle_task.unwrap();
                    services().task_manager.borrow_mut().set_state(idle, Running);
                    let returned = kernel().switch_to_task(idle);
                    continue;  // skip to next iteration after switching back to idle
                }
            };

            services().task_manager.borrow_mut().set_state(next_handle, Running);
            self.algorithm.reset_quantum_for(next_handle, current_priority);  // NEW METHOD? Or handle inside engine

            let returned_handle = kernel().switch_to_task(next_handle);

            let task_state = services().task_manager.borrow().get_state(returned_handle);
            
            match task_state {
                Running => {
                    services().task_manager.borrow_mut().set_state(returned_handle, Ready);
                    // Special-case: idle task doesn't get requeued into the algorithm's queues
                    if Some(returned_handle) != self.idle_task {
                        let yield_reason = services().task_manager.borrow().get_yield_reason(returned_handle);
                        self.algorithm.requeue_after_run(returned_handle, current_priority, yield_reason);
                    } else {
                        self.idle_task = Some(returned_handle);
                    }
                }
                Terminated => {
                    self.cleanup_completion_future(returned_handle);
                    services().task_manager.borrow_mut().remove_task(returned_handle);
                }
                Created | Ready | Blocked => {}  // No action needed
            }
        }
    }
}
```

**New observation:** The MLFQ strategy needs to know the yield reason to decide queue placement. Currently it calls `services().task_manager.borrow().get_yield_reason(handle)` inside its own methods. In the new design:

- **Option 1 (recommended):** Pass yield reason as a parameter alongside priority
- **Option 2:** Have the engine call `algorithm.set_context(handle, yield_reason, priority)` before each task runs, then `requeue_after_run` reads it internally

I recommend **Option 1** — it keeps strategies stateless and makes the contract explicit. The trait becomes:

```rust
pub trait SchedulingAlgorithm {
    fn pick_next(&mut self) -> Option<TaskHandle>;
    fn requeue_after_run(&mut self, handle: TaskHandle, current_priority: usize, yield_reason: YieldReason);
    fn push_ready(&mut self, handle: TaskHandle);
    fn should_preempt(&mut self) -> bool;

    // Optional — called when a task starts running (for quantum reset etc.)
    fn on_task_start(&mut self, _handle: TaskHandle, _priority: usize) {}  // default no-op
}
```

Wait — this is getting complex. Let me reconsider whether `requeue_after_run` should read the yield reason from services directly (as it currently does in both FIFO and MLFQ). 

**Revised approach:** Keep the trait simple. The engine handles all external state access; the strategy only manages its own queues:

```rust
pub trait SchedulingAlgorithm {
    fn pick_next(&mut self) -> Option<TaskHandle>;
    
    /// Requeue a task after it returns from running.
    /// `current_priority` is the priority level at which this task was executing.
    /// The strategy decides where to place it based on internal state and the engine's knowledge
    /// (the engine can call additional methods or set state before calling this).
    fn requeue_after_run(&mut self, handle: TaskHandle, current_priority: usize);
    
    fn push_ready(&mut self, handle: TaskHandle);
    fn should_preempt(&mut self) -> bool;

    /// Called by the engine when a task is about to run (for quantum reset in MLFQ).
    fn on_task_start(&mut self, priority: usize);
}
```

The engine's `run()` would look like:

```rust
let next_handle = match self.algorithm.pick_next() {
    Some(handle) => handle,
    None => self.idle_task.unwrap(),  // or continue loop logic
};

// For MLFQ, we need to know which queue the task came from.
// The engine tracks this via a helper: pick_next returns (handle, priority).
let (next_handle, current_priority) = match self.algorithm.pick_with_priority() {
    Some((h, p)) => (h, p),
    None => ...,
};

self.algorithm.on_task_start(current_priority);  // MLFQ resets quantum here
services().task_manager.borrow_mut().set_state(next_handle, Running);
let returned = kernel().switch_to_task(next_handle);

// ... state handling ...
Running => {
    set_state(returned, Ready);
    if Some(returned) != self.idle_task {
        let yield_reason = get_yield_reason(returned);
        // MLFQ uses yield_reason to decide queue; FIFO ignores it.
        // We need a way to communicate this to the strategy.
        self.algorithm.requeue_after_run_with_reason(returned, current_priority, yield_reason);
    } else {
        self.idle_task = Some(returned);
    }
}
```

Hmm, adding `requeue_after_run_with_reason` is ugly. Let me think of a cleaner way...

**Cleanest approach — store the yield reason in the strategy before requeuing:**

Add a method `record_yield(handle, reason)` to the trait that the engine calls right after the switch:

```rust
pub trait SchedulingAlgorithm {
    fn pick_next(&mut self) -> Option<TaskHandle>;
    fn record_yield(&mut self, handle: TaskHandle, current_priority: usize, yield_reason: YieldReason);
    fn requeue_after_run(&mut self, handle: TaskHandle);  // uses stored info from record_yield
    fn push_ready(&mut self, handle: TaskHandle);
    fn should_preempt(&mut self) -> bool;
    fn on_task_start(&mut self, priority: usize);
}
```

Then the engine's loop is clean:

```rust
let (handle, priority) = match self.algorithm.pick_next() { ... };
self.algorithm.on_task_start(priority);
// switch_to_task...
match state {
    Running => {
        set_state(handle, Ready);
        if handle != idle_task {
            let reason = get_yield_reason(handle);
            self.algorithm.record_yield(handle, priority, reason);
            self.algorithm.requeue_after_run(handle);  // reads stored reason internally
        } else {
            self.idle_task = Some(handle);
        }
    }
    ...
}
```

This is clean for FIFO (which ignores yield_reason and just pushes to back) and MLFQ (which uses the stored reason + priority). Both strategies only need to manage their own internal state.

---

### Step 5: Implement `Scheduler` engine struct (`scheduler_engine.rs`)

Create `kernel/src/scheduler/scheduler_engine.rs`:

```rust
use alloc::collections::VecDeque;
use crate::task::{TaskHandle, TaskState};
use super::algorithm::SchedulingAlgorithm;
use system::future::FutureHandle;
use crate::messages::HardwareInterrupt;
use crate::kernel::kernel;
use crate::kernel_services::services;
use crate::future::TaskFuture;

pub struct Scheduler {
    algorithm: Box<dyn SchedulingAlgorithm>,
    blocked_tasks: VecDeque<TaskFuture>,
    hw_interrupt_queue: VecDeque<HardwareInterrupt>,
    idle_task: Option<TaskHandle>,
}

impl Scheduler {
    pub fn new(algorithm: impl SchedulingAlgorithm + 'static) -> Self { ... }

    // === Public API (replaces old trait methods) ===
    pub fn run(&mut self);
    pub fn push_task(&mut self, handle: TaskHandle);
    pub fn push_blocked(&mut self, task_handle: TaskHandle, future_handle: FutureHandle);
    pub fn push_hardware_interrupt(&mut self, interrupt: HardwareInterrupt);
    pub fn set_idle_task(&mut self, handle: TaskHandle) -> Result<(), ()>;
    pub fn should_preempt(&mut self) -> bool;  // forwards to algorithm

    // === Shared logic (one copy total) ===
    fn process_hardware_interrupts(&mut self);
    fn poll_futures(&mut self);
    fn cleanup_completion_future(&mut self, task_handle: TaskHandle);
}
```

**`run()` implementation** — the orchestrator that delegates to the algorithm for task selection and requeueing. Idle task handling is centralized here (one copy).

---

### Step 6: Update `mod.rs`

Replace current contents with:

```rust
pub mod fifo_strategy;
pub mod mlfq_strategy;
mod timer;

mod algorithm;
mod scheduler_engine;

// Re-export for external use
pub use scheduler_engine::Scheduler;
pub use algorithm::SchedulingAlgorithm;

// Factory functions (can stay the same from caller perspective)
pub type SchedulerFactory = fn() -> Box<dyn SchedulingAlgorithm>;

pub fn mfq_scheduler() -> Box<dyn SchedulingAlgorithm> {
    Box::new(mlfq_strategy::MlfqStrategy::new())
}

pub fn fifo_scheduler() -> Box<dyn SchedulingAlgorithm> {
    Box::new(fifo_strategy::FifoStrategy::default())
}
```

**Note:** The factory returns `Box<dyn SchedulingAlgorithm>` not `Scheduler`. The kernel would construct the engine after:
```rust
let algorithm = mfq_scheduler();  // Box<dyn SchedulingAlgorithm>
let scheduler = Scheduler::new(algorithm);
```

Or, a combined factory could exist:
```rust
pub fn create_mfq() -> Scheduler {
    Scheduler::new(mlfq_strategy::MlfqStrategy::new())
}
```

---

### Step 7: Update callers outside `scheduler/` module

Files that reference the old `Scheduler` trait or construct schedulers directly will need updates:
- `kernel/src/main_thread.rs` — likely constructs and runs a scheduler
- Any test harnesses in other modules

Check for references to:
- `FifoScheduler::new()` → use `create_fifo()` factory instead
- `MlfqScheduler::new()` → use `create_mfq()` factory instead
- `.should_preempt()` calls on schedulers — still works, now forwards through engine

---

### Step 8: Update tests

**For strategy files (`fifo_strategy.rs`, `mlfq_strategy.rs`):**
- Move existing unit tests that test strategy-specific behavior (priority queues, quantum countdown, pick_next) into the respective strategy files
- Remove tests for shared methods (those belong on the engine mock)

**Create a fake strategy for engine tests:**

```rust
// Inside scheduler_engine.rs or a separate test module
#[cfg(test)]
mod tests {
    use super::*;
    
    struct FakeAlgorithm {
        pub next: Option<TaskHandle>,
        pub requeued: VecDeque<TaskHandle>,
    }

    impl SchedulingAlgorithm for FakeAlgorithm {
        fn pick_next(&mut self) -> Option<TaskHandle> { self.next.take() }
        fn record_yield(&mut self, _: TaskHandle, _: usize, _: YieldReason) {}
        fn requeue_after_run(&mut self, handle: TaskHandle) {
            self.requeued.push_back(handle);
        }
        fn push_ready(&mut self, handle: TaskHandle) { ... }
        fn should_preempt(&mut self) -> bool { false }
        fn on_task_start(&mut self, _: usize) {}
    }

    // Tests for Scheduler engine logic (interrupts, futures, cleanup, idle fallback)
}
```

**Tests to write for the engine:**
1. `poll_futures` correctly unblocks completed tasks and passes them to the algorithm's `push_ready`
2. `process_hardware_interrupts` processes keyboard events
3. `cleanup_completion_future` removes orphaned futures
4. When `pick_next` returns None, idle task runs instead
5. Idle task is not passed back through `requeue_after_run`

---

## Step-by-Step Execution Order

1. **Create `algorithm.rs`** — define the trait only (no impls yet)
2. **Create `fifo_strategy.rs`** — implement FifoStrategy with moved logic from fifo_scheduler.rs, keep existing tests
3. **Create `mlfq_strategy.rs`** — implement MlfqStrategy with moved logic, keeping priority tracking and quantum management; move relevant tests
4. **Create `scheduler_engine.rs`** — build the engine struct with all shared methods, use a placeholder strategy for compilation
5. **Update `mod.rs`** — wire up module declarations, factory functions
6. **Replace old files:** Delete `fifo_scheduler.rs` and `mlfq_scheduler.rs` (after confirming new files compile)
7. **Fix callers outside scheduler/** — update any code that directly instantiates or uses the old types
8. **Write engine tests** with a fake strategy to verify shared logic works correctly
9. **Run all tests** — ensure nothing is broken
10. **Build with `cargo build` in `arch/x86_64`** — verify full compilation

---

## Files Summary (After)

| File | Purpose | Estimated Lines |
|---|---|---|
| `algorithm.rs` | SchedulingAlgorithm trait definition | ~30 |
| `fifo_strategy.rs` | FifoStrategy impl + tests | ~80 |
| `mlfq_strategy.rs` | MlfqStrategy impl + tests | ~150 |
| `scheduler_engine.rs` | Scheduler struct (engine) with shared logic + engine tests | ~200 |
| `mod.rs` | Module declarations, re-exports, factory functions | ~30 |

**Total: ~490 lines** vs. current **~650 lines** — net reduction of ~160 lines by eliminating duplication.

---

## Risks & Mitigations

| Risk | Mitigation |
|---|---|
| Engine's `run()` loop becomes complex with priority tracking | Keep the engine focused on orchestration only; all scheduling decisions stay in strategies |
| Trait object overhead (vtable dispatch) | Negligible — one indirect call per task switch iteration. Can optimize later if needed |
| Test migration confusion (which tests go where?) | Strategy tests test algorithm-specific behavior; engine tests use fake strategies to isolate shared logic |
| ABI compatibility during refactoring | This is an in-tree refactor, no external API changes for callers outside the kernel — just type name changes |

---

## Open Design Decisions (to confirm before implementation)

1. **`pick_next` return value:** Returns `Option<TaskHandle>` only. Priority tracking is handled by a separate mechanism (`on_task_start` + `record_yield`). Confirm this feels right, or prefer `pick_next` to return `(TaskHandle, usize)` for priority alongside the handle.

2. **Factory pattern:** Should factories return `Scheduler` (engine) directly or `Box<dyn SchedulingAlgorithm>`? Recommend returning the engine from factory functions so callers don't need two-step construction.

3. **`should_preempt` forwarding:** The kernel's timer interrupt handler calls this. Confirm it currently does, and that after refactoring, the call site will still work by calling a method on `Scheduler` (the engine) that forwards to the strategy.

4. **Module naming:** Should the engine live in its own file (`scheduler_engine.rs`) or be defined directly inside `mod.rs`? For clarity with ~200 lines of code, I recommend the separate file.

5. **The old `Scheduler` trait name:** After this refactor, there's no trait named `Scheduler`. The engine is a struct called `Scheduler`, and the trait is `SchedulingAlgorithm`. Confirm this naming feels intuitive.
