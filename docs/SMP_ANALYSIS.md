# RosX SMP Support Analysis Report

This report outlines the critical parts of the RosX kernel that prevent it from supporting multiple processors (SMP) and provides a roadmap for implementation.

## 1. Executive Summary
The current RosX kernel is strictly single-core. Its synchronization relies on disabling interrupts and `KernelCell` (which only prevents compiler reordering). To support SMP, the kernel must transition to hardware-backed synchronization (Spinlocks) and move from a global execution state to per-CPU local storage.

## 2. Critical Bottlenecks

### A. Global Execution State
The `Kernel` and `ExecutionState` structs are currently global singletons.
*   **The Issue:** `ExecutionState` tracks the `current_task`, `preemption_enabled`, and the `cpu` pointer. In an SMP system, every CPU is running a different task and has its own preemption state.
*   **Requirement:** `ExecutionState` must become per-CPU.

### B. Synchronization Primitives
*   **`KernelCell`:** Currently used for almost all shared state. It provides NO protection against concurrent access from multiple cores. It only uses `compiler_fence`.
*   **Interrupt Disabling:** The kernel often uses `cpu.disable_interrupts()` to protect critical sections. This only stops the *local* CPU from being interrupted; it does nothing to stop another CPU from accessing the same data simultaneously.
*   **Requirement:** Implementation of a `Spinlock` that uses atomic instructions (PAUSE/CMPXCHG) and disables local interrupts.

### C. Global Memory Manager
*   **The Issue:** `MEMORY_MANAGER` is a global static. Its `alloc` and `dealloc` methods use local interrupt disabling.
*   **Requirement:** The allocator must be protected by a `Spinlock`. To avoid performance bottlenecks, per-CPU allocation caches (magazines) should eventually be considered.

### D. Scheduler and Task Management
*   **The Issue:** `Scheduler` and `TaskManager` are global.
*   **Requirement:** While `TaskManager` can remain global (protected by a lock), the `Scheduler` should ideally become per-CPU to allow for independent scheduling decisions and better scalability.

## 3. Required Kernel Abstractions

### A. Per-CPU Local Storage
The kernel needs a portable way to access data unique to the current processor.
*   **Mechanism:** A trait-based interface where the architecture provides a pointer to a `CpuControlBlock` (containing the `ExecutionState`). On x86_64, this usually uses the `gs` or `fs` segments.

### B. The `Lock` Trait
To support the user's requirement for pluggable/software-based solutions:
*   **`Spinlock<T>`:** Uses hardware atomics.
*   **`InterruptLock<T>`:** (For single-core) only disables interrupts.
*   **`SoftwareLock<T>`:** (e.g., Bakery Algorithm) for CPUs without atomics.

## 4. Architectural Requirements (HAL Updates)

The `Cpu` trait must be expanded to support:
1.  **`cpu_id()`**: Returns a unique identifier for the current processor.
2.  **`send_ipi(target, ipi)`**: Inter-Processor Interrupts are necessary for:
    *   Waking up an idle CPU when a task becomes ready.
    *   Forcing a reschedule on another CPU.
    *   TLB shootdowns (for future MMU support).
3.  **AP Bootstrapping**: A method for the Bootstrap Processor (BSP) to start the Application Processors (APs).

## 5. Interrupt Routing Comparison

| Strategy | Pros | Cons |
| :--- | :--- | :--- |
| **Pinned Interrupts** (Recommended) | Simple to implement, good cache locality for drivers. | Potential for load imbalance if one CPU is overwhelmed by IRQs. |
| **Global/Round-Robin Routing** | Better load balancing across CPUs. | Complex hardware configuration (IOAPIC/MSI), cache bouncing. |

**Recommendation:** Start with **Pinned Interrupts**. Assign the system timer to all CPUs (for preemption) but pin hardware devices (keyboard, disk) to CPU 0 for simplicity.

## 6. Proposed Implementation Roadmap

1.  **Phase 1: HAL Extensions.** Update `Cpu` trait with `cpu_id` and basic IPI support.
2.  **Phase 2: Synchronization Upgrade.** Replace `KernelCell` with a `Spinlock` abstraction.
3.  **Phase 3: Per-CPU State.** Move `ExecutionState` from `Kernel` to a per-CPU local storage mechanism.
4.  **Phase 4: Multicore Boot.** Implement the architecture-specific AP wakeup and common kernel entry point.
5.  **Phase 5: Scheduler Refactoring.** Transition from a single global scheduler to per-CPU schedulers with a task-migration mechanism.
