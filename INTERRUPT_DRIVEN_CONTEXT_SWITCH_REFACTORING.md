# Interrupt-Driven Context Switch Refactoring Plan

## Overview

Refactor the OS from `ret`-based context switching to a uniform interrupt-driven approach using `iretq`. This enables preemptive multitasking and provides a consistent mechanism for all context switches.

## Current State

- **Task initialization** (`initialize_task_for_swap`): Sets up stack frame for `swap_context` using `ret`
- **Context switching** (`swap_context`): Saves/restores 15 GPRs and uses `ret` to jump to tasks
- **Task yield**: Calls `swap_context` directly from Rust code
- **Initial task entry** (`restore_context`): Uses manual stack manipulation and `ret`

## Problem

The current implementation:
- Cannot support preemptive multitasking (no interrupt-driven switches)
- Has multiple code paths for context switching
- Not compatible with hardware interrupts (timer, keyboard, etc.)

## Goal

Achieve uniform interrupt-driven context switching where:
- All switches go through interrupts → `iretq`
- Voluntary yields use software interrupt (INT 0x30)
- Hardware interrupts (timer, keyboard) can preempt tasks
- Task initialization uses fake interrupt frame
- Single, consistent exit path for all context switches

---

## Implementation Steps

### ✓ = Done | ⧗ = In Progress | ☐ = Pending

### Phase 1: Foundation (Steps 1-4)

#### ☐ Step 1: Define Software Interrupt Numbers
**Status:** Pending

**Goal:** Allocate interrupt vectors and register them in the IDT

**Tasks:**
- Define interrupt numbers:
  - `YIELD_INTERRUPT = 0x30` - Voluntary task yielding
  - `TASK_INIT_INTERRUPT = 0x31` - First-time task entry (if needed)
- Add to `InterruptIndex` enum in `arch/x86_64/src/interrupts.rs`
- Register handlers in IDT initialization

**Files to modify:**
- `arch/x86_64/src/interrupts.rs`

**Notes:**
- Software interrupts can use any vector from 0x20-0xFF (avoid 0x00-0x1F for exceptions)
- 0x30 is a good choice as it's well above hardware IRQs (0x20-0x2F)

---

#### ☐ Step 2: Create Unified Interrupt Stack Frame Layout
**Status:** Pending

**Goal:** Document and define constants for the interrupt stack frame structure

**Stack Frame Layout (iretq-compatible):**
```
Offset   Size   Register/Field
------   ----   --------------
   0       8    r15
   8       8    r14
  16       8    r13
  24       8    r12
  32       8    r11
  40       8    r10
  48       8    r9
  56       8    r8
  64       8    rdi
  72       8    rsi
  80       8    rbp
  88       8    rdx
  96       8    rcx
 104       8    rbx
 112       8    rax
 120       8    RIP   (pushed by CPU on interrupt)
 128       8    CS    (pushed by CPU on interrupt)
 136       8    RFLAGS (pushed by CPU on interrupt)
[144       8    RSP]   (only if privilege level changes)
[152       8    SS]    (only if privilege level changes)
------
Total: 144 bytes (same privilege) or 160 bytes (privilege change)
```

**Tasks:**
- Create constants in assembly files:
  - `INTERRUPT_FRAME_SIZE = 144` (for same-privilege kernel)
  - `GPR_SAVE_SIZE = 120` (15 registers × 8 bytes)
- Document layout in both `context_switching.S` and `process_initialization.S`
- Define RFLAGS initial value: `RFLAGS_INIT = 0x202` (IF=1, reserved bit always 1)

**Files to modify:**
- `arch/x86_64/src/context_switching.S`
- `arch/x86_64/src/process_initialization.S`

---

#### ☐ Step 3: Rewrite Task Initialization
**Status:** Pending

**Goal:** Modify `initialize_task_for_swap` to create a fake interrupt frame instead of a `ret` frame

**Current behavior:**
- Creates frame for `swap_context` with return address at top
- Uses `ret` instruction to jump to task

**New behavior:**
- Creates frame that looks like an interrupt occurred
- CPU will use `iretq` to enter task
- Frame includes RIP, CS, RFLAGS (in addition to GPRs)

**Implementation details:**
```assembly
initialize_task_for_interrupt:
    # RDI: Stack top
    # RSI: Entry point (task wrapper)
    # RDX: Parameter for entry point (actual task entry)

    # Align and reserve space
    mov rax, rdi
    and rax, ~0xF
    sub rax, 144  # INTERRUPT_FRAME_SIZE

    # Initialize all GPRs to 0
    mov qword ptr [rax + 0], 0     # r15
    ...
    mov qword ptr [rax + 64], rdx  # rdi = entry_param
    ...
    mov qword ptr [rax + 112], 0   # rax

    # Build interrupt frame (what CPU pushes)
    mov qword ptr [rax + 120], rsi  # RIP = entry point

    # Get current CS
    mov rbx, 0
    mov bx, cs
    mov qword ptr [rax + 128], rbx  # CS

    # Set RFLAGS with interrupts enabled
    mov qword ptr [rax + 136], 0x202  # RFLAGS (IF=1)

    ret
```

**Files to modify:**
- `arch/x86_64/src/process_initialization.S`

**Files to update references:**
- `arch/x86_64/src/cpu.rs` (rename function or create new one)
- `kernel/src/kernel.rs` (update calls)

---

#### ☐ Step 4: Create Common Interrupt Return Path
**Status:** Pending

**Goal:** Write assembly function that all context switches exit through

**Implementation:**
```assembly
.global interrupt_return
interrupt_return:
    # RSP points to saved context (r15 at top)

    # Restore all general-purpose registers
    pop r15
    pop r14
    pop r13
    pop r12
    pop r11
    pop r10
    pop r9
    pop r8
    pop rdi
    pop rsi
    pop rbp
    pop rdx
    pop rcx
    pop rbx
    pop rax

    # Stack now has: RIP, CS, RFLAGS [, RSP, SS]
    # Let iretq restore them and jump to task
    iretq
```

**Files to modify:**
- `arch/x86_64/src/context_switching.S`

**Notes:**
- This is the **single exit point** for all context switches
- Never returns normally
- `iretq` pops RIP, CS, RFLAGS and jumps (and RSP, SS if privilege changed)

---

### Phase 2: Core Refactoring (Steps 5-7)

#### ☐ Step 5: Implement Yield Interrupt Handler
**Status:** Pending

**Goal:** Create interrupt handler that performs context switch on voluntary yield

**Rust handler skeleton:**
```rust
extern "x86-interrupt" fn yield_interrupt_handler(_stack_frame: InterruptStackFrame) {
    // At this point, CPU has pushed RIP, CS, RFLAGS
    // We need to:
    // 1. Save all GPRs to current task's stack
    // 2. Save stack pointer to current task
    // 3. Call scheduler to pick next task
    // 4. Load next task's stack pointer
    // 5. Jump to interrupt_return (which will iretq)

    // This will likely need to be implemented in assembly
    // Or use inline asm from Rust
}
```

**Assembly implementation approach:**
```assembly
.global yield_interrupt_handler_asm
yield_interrupt_handler_asm:
    # CPU has already pushed: RIP, CS, RFLAGS
    # Save all GPRs
    push rax
    push rbx
    push rcx
    push rdx
    push rbp
    push rsi
    push rdi
    push r8
    push r9
    push r10
    push r11
    push r12
    push r13
    push r14
    push r15

    # RSP now points to complete interrupt frame
    # Call Rust function to handle context switch
    mov rdi, rsp  # Pass stack pointer as argument
    call yield_handler_rust

    # yield_handler_rust returns new stack pointer in RAX
    mov rsp, rax

    # Jump to common return path
    jmp interrupt_return
```

**Rust helper:**
```rust
#[no_mangle]
extern "C" fn yield_handler_rust(current_sp: usize) -> usize {
    unsafe {
        // Save current task's SP
        if let Some(mut current_task) = CURRENT_TASK.take() {
            current_task.set_stack_pointer(current_sp);
            // Store back or pass to scheduler
        }

        // Get next task from scheduler
        // ...

        // Return next task's stack pointer
        next_task.stack_pointer()
    }
}
```

**Files to create/modify:**
- `arch/x86_64/src/interrupts.rs` (Rust handler registration)
- `arch/x86_64/src/context_switching.S` (assembly handler)

---

#### ☐ Step 6: Modify task_yield() API
**Status:** Pending

**Goal:** Change from function call to software interrupt

**Current implementation:**
```rust
pub fn task_yield() {
    // ... complex swap_context logic ...
}
```

**New implementation:**
```rust
pub fn task_yield() {
    unsafe {
        core::arch::asm!("int 0x30", options(noreturn));
    }
}
```

**Files to modify:**
- `kernel/src/kernel.rs`

**Notes:**
- Much simpler!
- `options(noreturn)` tells compiler this never returns normally
- Actually it does "return" but via scheduler putting us back on CPU later

---

#### ☐ Step 7: Update Kernel::start()
**Status:** Pending

**Goal:** Use interrupt return instead of swap_context for initial switch

**Current approach:**
```rust
self.cpu.swap_context(&mut dummy_sp, self.main_thread_task.stack_pointer());
```

**New approach:**
```rust
// Initialize main thread task with interrupt frame
let new_sp = self.cpu.initialize_task_for_interrupt(
    self.main_thread_task.stack_pointer(),
    self.main_thread_task.entry_point(),
    self.main_thread_task.actual_entry_point()
);
self.main_thread_task.set_stack_pointer(new_sp);

// Jump directly to interrupt return path
unsafe {
    core::arch::asm!(
        "mov rsp, {stack_ptr}",
        "jmp interrupt_return",
        stack_ptr = in(reg) new_sp,
        options(noreturn)
    );
}
```

**Files to modify:**
- `kernel/src/kernel.rs` (Kernel::start method)

---

### Phase 3: Preemption Support (Steps 8-9)

#### ☐ Step 8: Extend Keyboard Interrupt Handler
**Status:** Pending

**Goal:** Make keyboard interrupt use new context switch mechanism

**Current implementation:**
```rust
extern "x86-interrupt" fn keyboard_interrupt_handler(_stack_frame: InterruptStackFrame) {
    // ... read scancode ...
    // Send EOI
    kernel::kernel::task_yield();  // This now uses INT 0x30
}
```

**New approach - Option A (keep it simple):**
- Keep calling `task_yield()` which now triggers INT 0x30
- This means nested interrupts (keyboard → yield interrupt)

**New approach - Option B (direct switch):**
```rust
extern "x86-interrupt" fn keyboard_interrupt_handler(_stack_frame: InterruptStackFrame) {
    // ... read scancode ...
    // Send EOI

    // Directly perform context switch (similar to yield handler)
    unsafe {
        core::arch::asm!(
            "push rax",
            "push rbx",
            // ... push all GPRs ...
            "mov rdi, rsp",
            "call keyboard_context_switch_rust",
            "mov rsp, rax",
            "jmp interrupt_return",
            options(noreturn)
        );
    }
}
```

**Decision needed:**
- Option A is simpler but has nested interrupt overhead
- Option B is more efficient but duplicates code

**Files to modify:**
- `arch/x86_64/src/interrupts.rs`

---

#### ☐ Step 9: Add Timer Interrupt Handler
**Status:** Pending

**Goal:** Implement true preemptive multitasking with timer

**Steps:**
1. Configure timer (PIT or APIC timer)
2. Register timer interrupt handler in IDT
3. Handler performs context switch (like keyboard)
4. Unmask timer interrupt in PIC

**Implementation:**
```rust
extern "x86-interrupt" fn timer_interrupt_handler(_stack_frame: InterruptStackFrame) {
    // Increment tick counter
    // Send EOI

    // Every N ticks, perform context switch
    static mut TICK_COUNT: u64 = 0;
    unsafe {
        TICK_COUNT += 1;
        if TICK_COUNT >= TIME_SLICE_TICKS {
            TICK_COUNT = 0;
            // Perform context switch
            task_yield(); // or direct switch
        }
    }
}
```

**Configuration:**
```rust
pub fn setup_timer(frequency_hz: u32) {
    // Configure PIT channel 0
    // ...
}
```

**Files to modify:**
- `arch/x86_64/src/interrupts.rs`
- May need new file: `arch/x86_64/src/timer.rs`

---

### Phase 4: Cleanup (Steps 10-11)

#### ☐ Step 10: Remove Obsolete Code
**Status:** Pending

**Goal:** Delete old context switching implementation

**Functions to remove:**
- `swap_context` (assembly in `context_switching.S`)
- `restore_context` (assembly in `context_switching.S`)
- `initialize_task_for_swap` (assembly in `process_initialization.S`)
- `initialize_process_stack` (if unused)

**Files to modify:**
- `arch/x86_64/src/context_switching.S`
- `arch/x86_64/src/process_initialization.S`
- `arch/x86_64/src/cpu.rs` (remove extern declarations)

---

#### ☐ Step 11: Update CPU Trait
**Status:** Pending

**Goal:** Modernize CPU trait to reflect new architecture

**Old trait:**
```rust
pub trait Cpu {
    fn swap_context(&self, store: *mut usize, load: usize);
    fn switch_to(&self, sp: usize) -> !;
    fn initialize_task(&self, sp: usize, entry: usize, param: usize) -> usize;
}
```

**New trait:**
```rust
pub trait Cpu {
    fn setup(&self);
    fn enable_interrupts(&self);
    fn disable_interrupts(&self);
    fn setup_sys_ticks(&self);
    fn initialize_task_for_interrupt(&self, sp: usize, entry: usize, param: usize) -> usize;
    // swap_context and switch_to removed - now done via interrupts
}
```

**Files to modify:**
- `kernel/src/cpu.rs` (trait definition)
- `arch/x86_64/src/cpu.rs` (implementation)

---

### Phase 5: Testing (Steps 12-15)

#### ☐ Step 12: Test Voluntary Yielding
**Status:** Pending

**Test cases:**
1. Single task that yields - should return to main thread
2. Two tasks yielding back and forth
3. Multiple yields from same task
4. Yield from task that will terminate

**Validation:**
- Tasks resume with correct register values
- Stack pointer remains in valid range
- No memory corruption
- Proper task state transitions

---

#### ☐ Step 13: Test Task Initialization
**Status:** Pending

**Test cases:**
1. Task with simple entry point
2. Task with entry point that takes parameters
3. Multiple tasks initialized before any run
4. Task that immediately yields

**Validation:**
- Entry point executes correctly
- Parameters passed correctly (RDI)
- Stack frame properly formed
- RFLAGS has correct value (interrupts enabled)

---

#### ☐ Step 14: Test Preemption with Keyboard
**Status:** Pending

**Test cases:**
1. Task running when keyboard interrupt occurs
2. Task in tight loop - keyboard should preempt
3. Rapid keyboard interrupts
4. Keyboard interrupt while handling another interrupt (nested)

**Validation:**
- Task resumes at correct instruction
- No register corruption
- Interrupt is handled properly
- EOI sent correctly

---

#### ☐ Step 15: Test Timer-Based Preemption
**Status:** Pending

**Test cases:**
1. Task that never yields - should be preempted by timer
2. Multiple CPU-bound tasks - should round-robin
3. Mix of yielding and non-yielding tasks
4. Very short time slices (stress test)

**Validation:**
- All tasks make progress
- Fair scheduling
- No deadlocks
- Proper time accounting

---

## Key Design Decisions

### Same-Privilege vs. Privilege Change
- **Decision:** Use same-privilege (kernel-only) for now
- **Rationale:** Simpler, smaller stack frames (144 vs 160 bytes)
- **Future:** Can extend to user mode later by including RSP/SS in frame

### Nested Interrupts
- **Decision:** TBD - allow or disable during handlers?
- **Option A:** Disable (simpler, safer)
- **Option B:** Allow (better latency, more complex)

### Context Switch from Interrupt
- **Decision:** Create dedicated assembly handlers
- **Rationale:** Rust's `x86-interrupt` ABI doesn't give enough control for context switch

### Software Interrupt Number
- **Decision:** 0x30 for yield
- **Rationale:** Well above hardware IRQs, conventional range for syscalls

---

## References

### Files in Project
- `arch/x86_64/src/context_switching.S` - Context switch assembly
- `arch/x86_64/src/process_initialization.S` - Task initialization assembly
- `arch/x86_64/src/interrupts.rs` - Interrupt handlers
- `arch/x86_64/src/cpu.rs` - CPU trait implementation
- `kernel/src/cpu.rs` - CPU trait definition
- `kernel/src/kernel.rs` - Kernel and task_yield API
- `kernel/src/task.rs` - Task structure

### x86_64 Interrupt Behavior
- CPU automatically pushes: SS, RSP, RFLAGS, CS, RIP (if privilege changes)
- Same privilege: Only RIP, CS, RFLAGS
- Handler must: Save GPRs, do work, restore GPRs, `iretq`
- `iretq` pops: RIP, CS, RFLAGS [, RSP, SS]

---

## Progress Tracking

**Last Updated:** 2025-10-11

**Current Phase:** Planning Complete

**Next Step:** Begin Phase 1, Step 1 (Define software interrupt numbers)
