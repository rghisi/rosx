# RosX Development Log

## Session: 2025-10-07 - Interrupt-Driven Task Switching

### Current State

Implemented interrupt-driven preemptive multitasking for the x86_64 kernel. The system now supports both cooperative task switching (when tasks complete) and preemptive switching (via keyboard interrupts).

### What Was Implemented

#### 1. Interrupt Infrastructure (`arch/x86_64/src/interrupts.rs`)
- **IDT Setup**: Interrupt Descriptor Table configured with keyboard handler
- **PIC Configuration**: 8259 PICs remapped to avoid conflicts with CPU exceptions
  - PIC1 offset: 0x20
  - PIC2 offset: 0x28
- **Interrupt Lifecycle**:
  1. `init()` called during kernel startup - loads IDT, initializes PICs, masks all interrupts
  2. `enable_interrupts()` called after task system is ready - enables CPU interrupts, unmasks keyboard IRQ1
- **Keyboard Handler**: Reads scancode, sends EOI to PIC, calls `task_yield()` for context switch

#### 2. Task State Management Updates (`kernel/src/task.rs`)
- **Task Wrapper**: Now marks tasks as terminated before yielding back to MainThread
- Ensures completed tasks don't get re-queued

#### 3. Main Thread Scheduling Logic (`kernel/src/main_thread.rs`)
- **Smart Re-queuing**:
  - Tasks that yielded while still Running → re-queued to ready list
  - Tasks that completed (terminated) → not re-queued
- **Interrupt Enablement**: Interrupts enabled at line 60 after task system initialization

#### 4. Architecture Integration (`arch/x86_64/src/cpu.rs`, `main.rs`)
- `enable_interrupts()` wired up to call interrupt subsystem
- Added `#![feature(abi_x86_interrupt)]` for interrupt handler ABI
- `interrupts::init()` called before kernel start

#### 5. Dependencies Added
```toml
x86_64 = "0.15"
pic8259 = "0.11"
```

### Test Configuration

Modified dummy tasks with longer delays to verify task switching:
- Idle task: delays 100,000,000 cycles between prints
- Job 1: 20,000,000 cycle delay
- Job 2: 20,000,000 cycle delay

### How It Works

1. **Startup Sequence**:
   - Kernel setup creates MainThread with idle task
   - Two function tasks scheduled (Job 1, Job 2)
   - `interrupts::init()` sets up IDT and PICs (interrupts still disabled)
   - `kernel.start()` begins MainThread execution

2. **MainThread Execution**:
   - Initializes task system, marks itself as Running
   - **Enables interrupts** (critical step at line 60)
   - Enters scheduling loop

3. **Task Scheduling Loop**:
   - Takes next ready task from queue
   - Swaps context to task
   - When returns: checks if task Running (yielded) or Terminated (completed)
   - Re-queues yielded tasks, discards terminated ones
   - If no ready tasks, runs idle task

4. **Interrupt-Driven Preemption**:
   - User presses key → keyboard interrupt (IRQ1)
   - `keyboard_interrupt_handler` invoked
   - Reads scancode, sends EOI
   - Calls `task_yield()` → context switches back to MainThread
   - Task stays in Running state → gets re-queued

### Key Files Modified

- `arch/x86_64/src/interrupts.rs` (new) - Interrupt subsystem
- `arch/x86_64/Cargo.toml` - Added x86_64 and pic8259 dependencies
- `arch/x86_64/src/main.rs` - Added feature flag, interrupts module, init call
- `arch/x86_64/src/cpu.rs` - Wired enable_interrupts()
- `kernel/src/main_thread.rs` - Smart re-queuing logic, interrupt enablement
- `kernel/src/task.rs` - Task wrapper marks tasks terminated
- `Cargo.lock` - Dependency updates

### Git Status

Modified files ready to commit:
```
M Cargo.lock
M arch/x86_64/Cargo.toml
M arch/x86_64/src/cpu.rs
M arch/x86_64/src/main.rs
M kernel/src/main_thread.rs
M kernel/src/task.rs
?? arch/x86_64/src/interrupts.rs
```

### Current Issues

**Task Finalization Handling Problem**: The system is currently stuck with improper handling of task finalization. When tasks complete, the task wrapper tries to mark them as terminated and yield back to MainThread, but there's an issue with how this finalization is being handled. This needs to be debugged and fixed before proceeding with other features.

### Next Steps / Potential Improvements

1. **FIX: Task Finalization** ⚠️ - Debug and fix the improper handling of task finalization when tasks complete
2. **Timer Interrupt**: Add timer (IRQ0) for regular preemption instead of keyboard-only
3. **Priority Scheduling**: Implement task priorities
4. **Sleep/Wake**: Add ability for tasks to sleep for specific time periods
5. **More Interrupt Handlers**: Handle other hardware interrupts (mouse, disk, etc.)
6. **Exception Handlers**: Add CPU exception handlers (page fault, divide by zero, etc.)
7. **Syscall Interface**: Add software interrupt for syscalls
8. **Testing**: Verify edge cases and stress test the scheduler

### Technical Notes

- **Safety**: Interrupts are masked during PIC initialization to prevent spurious interrupts
- **Ordering**: CPU interrupts enabled before unmasking PIC interrupts for clean initialization
- **Pending Data**: Keyboard port cleared before enabling to avoid stale scancodes
- **Context Preservation**: Stack frames properly preserved during interrupt handling
- **EOI Timing**: End of Interrupt sent before task yield to acknowledge hardware

### Recent Commits

```
624d504 Working task switching with idle task when no ready tasks
5d277ce Context switching step 1
eaa2b9b Task initialization and switch to working
5b5b64b Before Claude
```

### Build & Run

The system should build and run with the bootloader. Pressing any key should trigger task switching, visible in the debug output showing tasks being interrupted and re-queued.
