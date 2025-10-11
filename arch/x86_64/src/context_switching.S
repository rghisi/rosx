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
# swap_context: Save current context and switch to another task (IRETQ version)
# ============================================================================
# This is the interrupt-driven version that uses IRETQ instead of RET.
# It expects the stack to contain a full interrupt frame with IRET fields.
#
# Parameters:
#   RDI: Pointer to store current stack pointer (*mut usize)
#   RSI: Stack pointer to load (usize) - must point to an interrupt frame
#
# This function:
#   1. Saves all 15 GPRs to the current stack
#   2. Stores the current stack pointer
#   3. Loads the new stack pointer (which points to an interrupt frame)
#   4. Restores all GPRs from the new stack
#   5. Uses IRETQ to jump to the new task (popping RIP, CS, RFLAGS)
#
# NOTE: The caller must have set up an interrupt frame on the current stack
# BEFORE calling this function (RIP, CS, RFLAGS below the return address).
#
.global swap_context

swap_context:
    # Save parameters
    mov r10, rdi  # r10 = pointer to store current SP
    mov r11, rsi  # r11 = SP to load

    # Push all 15 general-purpose registers
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

    # RSP now points to the base of our saved context (r15 at top)
    # Save this stack pointer
    mov [r10], rsp

    # Load new stack pointer (points to an interrupt frame)
    mov rsp, r11

    # Restore all 15 general-purpose registers from the new stack
    # After this, stack will have: [RIP] [CS] [RFLAGS]
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

    # Use IRETQ to restore RIP, CS, RFLAGS and jump to the new task
    # This pops 3 values (or 5 if privilege changes) and jumps to RIP
    iretq

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
