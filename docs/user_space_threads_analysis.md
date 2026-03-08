# Analysis: User-Space Threads in RosX

This document outlines the requirements and design considerations for implementing user-space threads in RosX, supporting both applications and multi-threaded IPC servers.

## 1. Architectural Model: Process vs. Thread

Currently, RosX uses a flat `Task` model where each task is an independent unit of execution and resource ownership. To support threads that share data, we should transition to a hierarchical **Process-Thread** model.

### Model A: Resource-Sharing Tasks (Flat)
- **Concept:** Threads are just `Task`s that share a `ResourceGroup` ID.
- **Pros:** Minimal changes to the existing `TaskManager`.
- **Cons:** Harder to manage process-wide lifecycle (e.g., "kill all threads of process X").

### Model B: Process-Thread Hierarchy (Recommended)
- **Concept:** A `Process` is a container for one or more `Thread`s (which are specialized `Task`s).
- **Pros:**
    - Better isolation for future MMU support.
    - Centralized resource tracking (PID-based memory ownership).
    - Easier lifecycle management (cascading termination).
- **Cons:** Requires more significant refactoring of `kernel/src/task.rs` and `kernel/src/task_manager.rs`.

---

## 2. Kernel-Level Changes

### Data Structures
- **`Process` Struct:**
    ```rust
    pub struct Process {
        pub pid: usize,
        pub name: &'static str,
        pub threads: Vec<TaskHandle>,
        pub main_thread: TaskHandle,
        // Future: PageTable/AddressSpace
    }
    ```
- **`Task` Update:** Add `process_id: usize` to the `Task` struct to identify which process it belongs to.

### System Calls
- **`sys_spawn(entry: usize, param: usize) -> TaskHandle`**:
    - Creates a new `Task` within the *same* process context as the caller.
    - Assigns the same PID to the new task.
    - Initializes a new stack for the thread.
- **`sys_exit_process(code: i32)`**: Terminates all threads associated with the current PID.

### Memory Management
- **`MemoryManager` / `FreeListAllocator`**:
    - Update `BlockOwner::Task(usize)` to use the **PID** instead of a specific `TaskHandle` index.
    - This allows memory allocated by `Thread A` to be accessible and (if needed) freed by `Thread B` within the same process.
    - When a process terminates, `deallocate_by_owner(pid)` will reclaim all memory used by all its threads.

---

## 3. Userspace Changes (`usrlib`)

### Threading API
A high-level Rust-like API should be provided in `usrlib`:
```rust
pub mod thread {
    pub fn spawn<F>(f: F) -> JoinHandle
    where F: FnOnce() + Send + 'static
    {
        // 1. Allocate stack in userspace heap (optional, see below)
        // 2. Invoke sys_spawn
    }
}
```

### Stack Management
- **Option 1 (Kernel-side):** Kernel allocates a fixed-size stack for every `sys_spawn`.
    - *Pros:* Simple for userspace.
    - *Cons:* Wasteful if many threads are created with small stack needs.
- **Option 2 (User-side - Recommended):** `usrlib` allocates the stack from its heap and passes the pointer to the kernel.
    - *Pros:* Flexible stack sizes; userspace can recycle stacks.
    - *Cons:* `usrlib` must ensure stack cleanup (Join logic).

### Synchronization Primitives
To share data safely, we need:
- **`Mutex<T>`**: Uses a spinlock for short durations or a `sys_block`/`sys_wake` mechanism for long waits.
- **`Condvar`**: Allows threads to wait for specific conditions.
- **`Atomic`**: Utilize Rust's `core::sync::atomic` (already works if the CPU supports it).

---

## 4. Multi-Threaded IPC Servers

### Concurrency in `IpcManager`
- The current `IpcManager` is designed around a single mailbox per service.
- **Multiple Workers:** Multiple threads in an IPC server can call `ipc_receive` on the same service handle.
- **Synchronization:** The kernel's `IpcManager` must be thread-safe (currently handled by `KernelCell` and interrupt disabling, but may need finer-grained locking if SMP is added).

---

## 5. Lifecycle Management

- **Cascading Termination:** If the main thread of a process exits, the kernel should automatically mark all other threads with the same PID as `Terminated`.
- **Resource Reclaiming:** Upon the last thread of a PID exiting, the `TaskManager` should notify the `MemoryManager` to free all blocks owned by that PID.

---

## 6. MMU vs. MMU-less Considerations

- **Current (MMU-less):** All threads share the same physical address space. "Shared data" is just pointers to the same memory. No hardware protection prevents threads from interfering with other processes.
- **Future (With MMU):** The `Process` struct will own a Page Table. `sys_spawn` will create threads that share the same Page Table. This provides hardware-level isolation between different processes while allowing fast sharing between threads of the same process.

---

## Summary of Implementation Steps

1.  **Refactor `Task` and `TaskManager`** to support `PID`.
2.  **Update `MemoryManager`** to use `PID` for ownership tracking.
3.  **Implement `sys_spawn`** in the kernel.
4.  **Add `thread::spawn`** and basic **Join** logic to `usrlib`.
5.  **Implement `Mutex`** in `usrlib` using `yield` or a new `block` syscall.
6.  **Update IPC servers** to utilize worker thread pools.
