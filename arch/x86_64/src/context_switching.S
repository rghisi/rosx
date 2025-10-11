# --- Context Switching for x86_64 ---
# This file contains assembly functions for context switching.
#
# Stack Frame Layout (must match process_initialization.S):
# Offset   Size   Register/Field
# ------   ----   --------------
#    0       8    r15
#    8       8    r14
#   16       8    r13
#   24       8    r12
#   32       8    r11
#   40       8    r10
#   48       8    r9
#   56       8    r8
#   64       8    rdi
#   72       8    rsi
#   80       8    rbp
#   88       8    rdx
#   96       8    rcx
#  104       8    rbx
#  112       8    rax
#  120       8    RIP (IRET frame)
#  128       8    CS  (IRET frame)
#  136       8    RFLAGS (IRET frame)
#  144       8    RSP (IRET frame)
#  152       8    SS  (IRET frame)
# ------
# Total: 160 bytes (20 QWORDS)

.global swap_context

# swap_context: Save current context and switch to another task
# Parameters:
#   RDI: Pointer to store current stack pointer (*mut usize)
#   RSI: Stack pointer to load (usize)
#
# This function saves the current execution context (all registers + return address)
# to the current stack, stores the stack pointer, then loads the new stack pointer
# and restores the context from there.
#
# NOTE: This now saves ALL registers to support context switching from interrupts
swap_context:
    # Save parameters in registers we're about to push anyway
    # We'll store them in the stack frame in the right positions
    mov r10, rdi  # r10 = pointer to store current SP
    mov r11, rsi  # r11 = SP to load

    # Push all 15 general-purpose registers
    # NOTE: We push the ACTUAL values of r10/r11, not our saved parameters
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

    # Now RSP points to the base of our saved context (r15)
    # Save this stack pointer
    mov [r10], rsp

    # Load new stack pointer from r11
    mov rsp, r11

    # Restore all registers from the new stack
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

    # Return (to the new context)
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
