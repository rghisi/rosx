# --- Context Switching for x86_64 (INTERRUPT-DRIVEN VERSION) ---
# This file contains assembly functions for interrupt-driven context switching.
#
# Stack Frame Layout (IRETQ-compatible, must match process_initialization.S):
# This is the layout created by interrupt handlers and expected by interrupt_return.
# When the CPU takes an interrupt, it automatically pushes RIP, CS, RFLAGS.
# We save the GPRs on top of that.
#
# Offset   Size   Register/Field
# ------   ----   --------------
#    0       8    r15            ─┐
#    8       8    r14             │
#   16       8    r13             │
#   24       8    r12             │
#   32       8    r11             │
#   40       8    r10             │  General Purpose Registers
#   48       8    r9              │  (saved by our code)
#   56       8    r8              │
#   64       8    rdi             │
#   72       8    rsi             │
#   80       8    rbp             │
#   88       8    rdx             │
#   96       8    rcx             │
#  104       8    rbx             │
#  112       8    rax            ─┘
#  120       8    RIP            ─┐  IRET Frame
#  128       8    CS              │  (pushed by CPU on interrupt)
#  136       8    RFLAGS         ─┘
# ------
# Total: 144 bytes (same privilege switching)
#
# Note: RSP and SS would be at offsets 144 and 152 if privilege changes,
# but we're doing same-privilege (kernel-to-kernel) switches for now.

# ============================================================================
# restore_context_and_iretq: Restore registers and return via IRETQ
# ============================================================================
# This is the second half of swap_context, exposed as a separate entry point.
# It's used for the initial kernel->task switch where we don't need to save
# the kernel's context.
#
# Parameters:
#   RDI: Stack pointer to load (must point to an interrupt frame)
#
# This function does not return - it jumps to the task via IRETQ.
#
.global restore_context_and_iretq

# ============================================================================
# swap_context: Save current context and switch to another task (HYBRID version)
# ============================================================================
# This is a HYBRID function that:
#   - Saves the current context as if called from a normal function (GPRs only)
#   - Restores the target context as an interrupt frame (GPRs + IRETQ)
#
# This asymmetry is correct because:
#   - We're being CALLED as a normal Rust function (regular call/ret ABI)
#   - We're JUMPING TO a task that was suspended by an interrupt (needs IRETQ)
#
# After the bootstrap yield, the main_thread is running in "interrupt mode"
# (its saved state is an interrupt frame), but when it schedules tasks, it
# calls this function normally, not via INT.
#
# Parameters:
#   RDI: Pointer to store current stack pointer (*mut usize)
#   RSI: Stack pointer to load (usize) - must point to an interrupt frame
#
.global swap_context

swap_context:
    # Called as a normal function: [ret_addr] is at [RSP]
    # Parameters: RDI = *mut usize (where to store SP), RSI = usize (SP to load)
    #
    # Goal: Save current context (15 GPRs + return address), restore target's context

    # Step 1: Make space for full interrupt frame (144 bytes)
    # We include CS/RFLAGS to match task frame format, even though we use RET
    sub rsp, 144

    # Step 2: Save all 15 GPRs using direct memory writes
    mov [rsp + 0], r15
    mov [rsp + 8], r14
    mov [rsp + 16], r13
    mov [rsp + 24], r12
    mov [rsp + 32], r11
    mov [rsp + 40], r10
    mov [rsp + 48], r9
    mov [rsp + 56], r8
    mov [rsp + 64], rdi
    mov [rsp + 72], rsi
    mov [rsp + 80], rbp
    mov [rsp + 88], rdx
    mov [rsp + 96], rcx
    mov [rsp + 104], rbx
    mov [rsp + 112], rax

    # Step 3: Build IRET frame (even though we'll use RET)
    # Get return address (now at [RSP + 144])
    mov rax, [rsp + 144]
    mov [rsp + 120], rax        # RIP

    # Get CS and RFLAGS (even though RET won't use them, keep frame consistent)
    mov ax, cs
    movzx rax, ax
    mov [rsp + 128], rax        # CS

    pushfq
    pop rax
    mov [rsp + 136], rax        # RFLAGS

    # Step 4: Save current SP (RSP points to complete 144-byte frame)
    mov [rdi], rsp

    # Step 5: Load new SP (points to target's interrupt frame)
    mov rsp, rsi

    # Step 6: Restore all 15 GPRs from new interrupt frame
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

    # Step 7: Jump to target task
    # Use RET (pops only RIP) instead of IRETQ for now
    ret

restore_context_and_iretq:
    # This is a separate entry point for direct calls
    # Load the stack pointer from RDI parameter
    mov rsp, rdi

    # Restore all 15 general-purpose registers
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

    # Now stack has: [RIP] [CS] [RFLAGS]
    # TEMPORARY: Try using ret instead of iretq
    # This will just pop RIP and jump, ignoring CS and RFLAGS
    ret

# ============================================================================
# switch_to_task_interrupt_handler_asm: Switch from kernel to task (INT 0x31)
# ============================================================================
# Called when main_thread wants to switch to a task.
# CPU has already pushed: RIP, CS, RFLAGS (return point in main_thread)
# We save main_thread's GPRs, call Rust helper to get task SP, then switch to task.
#
.global switch_to_task_interrupt_handler_asm
switch_to_task_interrupt_handler_asm:
    # CPU has already pushed: RIP, CS, RFLAGS
    # Save all 15 general-purpose registers
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

    # RSP now points to a complete interrupt frame (r15 at top)
    # Call Rust helper function with current stack pointer as argument
    mov rdi, rsp
    call switch_to_task_handler_rust

    # switch_to_task_handler_rust returns task's stack pointer in RAX
    mov rsp, rax

    # Restore all 15 GPRs from task's stack
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

    ret

# ============================================================================
# yield_interrupt_handler_asm: Naked assembly yield interrupt handler
# ============================================================================
# This is the equivalent of a "naked" interrupt handler in C.
# Called when INT 0x30 is triggered. CPU has already pushed: RIP, CS, RFLAGS
# We save all GPRs, call Rust helper to pick next task, then switch to it.
#
# This function NEVER returns normally - it always uses IRETQ.
#
.global yield_interrupt_handler_asm
yield_interrupt_handler_asm:
    # CPU has already pushed: RIP, CS, RFLAGS
    # Save all 15 general-purpose registers in the order expected by our frame layout
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

    # RSP now points to a complete interrupt frame (r15 at top)
    # Call Rust helper function with current stack pointer as argument
    mov rdi, rsp
    call yield_handler_rust

    # yield_handler_rust returns new task's stack pointer in RAX
    mov rsp, rax

    # DEBUG: Print IRET frame values BEFORE popping (doesn't corrupt anything)
    mov rbx, [rsp + 120]  # RIP
    mov rcx, [rsp + 128]  # CS
    mov rdx, [rsp + 136]  # RFLAGS
    # Can't call print_hex_rax here without corrupting stack
    # Just note: RBX=RIP, RCX=CS, RDX=RFLAGS for debugging

    # Restore all 15 GPRs from new task's stack
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

    # Return via IRETQ (this handler was called by INT, so we must use IRETQ)
    iretq

# Helper function to print RAX in hex (preserves all registers except AL)
print_hex_rax:
    push rbx
    push rcx
    push rdx
    push rax

    mov rbx, rax
    mov rcx, 16  # 16 hex digits

.print_loop:
    rol rbx, 4
    mov al, bl
    and al, 0x0f
    cmp al, 10
    jl .is_digit
    add al, 'a' - 10
    jmp .print_char
.is_digit:
    add al, '0'
.print_char:
    out 0xe9, al
    loop .print_loop

    pop rax
    pop rdx
    pop rcx
    pop rbx
    ret
