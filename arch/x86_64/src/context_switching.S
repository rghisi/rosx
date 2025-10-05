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

.global restore_context

# restore_context: Restore a task's context and switch to it
# This function NEVER returns - it transfers control to the task
#
# Parameters:
#   RDI: Stack pointer to the saved context
#
# This function:
#   1. Loads the stack pointer
#   2. Pops all general-purpose registers
#   3. Executes iretq to restore CS, RIP, RFLAGS, SS, RSP and jump to the task
restore_context:
    # DEBUG: Check the stack state when entering
    push rdi                    # Save our argument

    # Print current RSP
    mov al, 'R'
    out 0xe9, al
    mov al, 'S'
    out 0xe9, al
    mov al, 'P'
    out 0xe9, al
    mov al, ':'
    out 0xe9, al
    mov rax, rsp
    call print_hex_rax
    mov al, ' '
    out 0xe9, al

    # Print what's at [RSP] (should be our saved RDI)
    mov al, '['
    out 0xe9, al
    mov al, 'R'
    out 0xe9, al
    mov al, 'S'
    out 0xe9, al
    mov al, 'P'
    out 0xe9, al
    mov al, ']'
    out 0xe9, al
    mov al, ':'
    out 0xe9, al
    mov rax, [rsp]
    call print_hex_rax
    mov al, ' '
    out 0xe9, al

    # Print what's at [RSP+8] (should be return address)
    mov al, 'R'
    out 0xe9, al
    mov al, 'E'
    out 0xe9, al
    mov al, 'T'
    out 0xe9, al
    mov al, ':'
    out 0xe9, al
    mov rax, [rsp + 8]
    call print_hex_rax
    mov al, 0x0a
    out 0xe9, al

    pop rdi                     # Restore our argument

    # DEBUG: Print task stack pointer
    mov al, 'T'
    out 0xe9, al
    mov al, 'S'
    out 0xe9, al
    mov al, 'P'
    out 0xe9, al
    mov al, ':'
    out 0xe9, al
    mov rax, rdi
    push rdi
    call print_hex_rax
    pop rdi
    mov al, 0x0a
    out 0xe9, al

    # Load the task's stack pointer
    mov rsp, rdi

    # DEBUG: Confirm we switched stacks and show what's at RSP
    push rax
    push rbx
    push rcx
    mov rbx, rsp
    mov al, 'S'
    out 0xe9, al
    mov al, 'W'
    out 0xe9, al
    mov al, 'I'
    out 0xe9, al
    mov al, 'T'
    out 0xe9, al
    mov al, 'C'
    out 0xe9, al
    mov al, 'H'
    out 0xe9, al
    mov al, ' '
    out 0xe9, al
    mov al, 'R'
    out 0xe9, al
    mov al, 'S'
    out 0xe9, al
    mov al, 'P'
    out 0xe9, al
    mov al, ':'
    out 0xe9, al
    mov rax, rbx
    add rax, 24  # Account for our 3 pushes
    call print_hex_rax
    mov al, ' '
    out 0xe9, al
    mov al, 'A'
    out 0xe9, al
    mov al, 'D'
    out 0xe9, al
    mov al, 'R'
    out 0xe9, al
    mov al, '1'
    out 0xe9, al
    mov al, '2'
    out 0xe9, al
    mov al, '0'
    out 0xe9, al
    mov al, ':'
    out 0xe9, al
    mov rcx, rbx
    add rcx, 24 + 120  # Our pushes + offset 120
    mov rax, rcx
    call print_hex_rax
    mov al, ' '
    out 0xe9, al
    mov al, 'V'
    out 0xe9, al
    mov al, 'A'
    out 0xe9, al
    mov al, 'L'
    out 0xe9, al
    mov al, ':'
    out 0xe9, al
    mov rax, [rcx]  # Read from that address
    call print_hex_rax
    mov al, 0x0a
    out 0xe9, al
    pop rcx
    pop rbx
    pop rax

    # Restore all general-purpose registers EXCEPT rax and rbx
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
    add rsp, 8  # Skip rbx (we'll use it for stack pointer)

    # Skip rax value
    add rsp, 8

    # DEBUG: Check where we are
    push rax
    mov al, 'P'
    out 0xe9, al
    mov al, 'O'
    out 0xe9, al
    mov al, 'P'
    out 0xe9, al
    mov al, 0x0a
    out 0xe9, al
    pop rax

    # Load entry point (RIP) into rax
    pop rax

    # DEBUG: Print entry point
    push rbx
    push rax
    mov al, 'E'
    out 0xe9, al
    mov al, 'N'
    out 0xe9, al
    mov al, 'T'
    out 0xe9, al
    mov al, ':'
    out 0xe9, al
    mov rbx, rax
    mov rax, rbx
    call print_hex_rax
    pop rax
    pop rbx
    push rax
    mov al, 0x0a
    out 0xe9, al
    pop rax

    # Load original stack top (stored in CS field) into rbx
    pop rbx

    # DEBUG: Print original stack top
    push rax
    mov al, 'S'
    out 0xe9, al
    mov al, 'T'
    out 0xe9, al
    mov al, 'K'
    out 0xe9, al
    mov al, ':'
    out 0xe9, al
    mov rax, rbx
    push rbx
    call print_hex_rax
    pop rbx
    mov al, 0x0a
    out 0xe9, al
    pop rax

    # Skip RFLAGS
    add rsp, 8

    # Set RSP to original stack top
    mov rsp, rbx

    # DEBUG: Confirm stack set
    push rax
    mov al, 'J'
    out 0xe9, al
    mov al, 'M'
    out 0xe9, al
    mov al, 'P'
    out 0xe9, al
    mov al, 0x0a
    out 0xe9, al
    pop rax

    # Push entry point and return to it (simulates a function call)
    push rax
    ret

.global swap_context

# swap_context: Save current context and switch to another task
# Parameters:
#   RDI: Pointer to store current stack pointer (*mut usize)
#   RSI: Stack pointer to load (usize)
#
# This function saves the current execution context (all registers + return address)
# to the current stack, stores the stack pointer, then loads the new stack pointer
# and restores the context from there.
swap_context:
    # Save all callee-saved registers and additional state
    push rbp
    push rbx
    push r12
    push r13
    push r14
    push r15

    # Save current stack pointer to the location pointed by RDI
    mov [rdi], rsp

    # Load new stack pointer from RSI
    mov rsp, rsi

    # Restore all registers from the new stack
    pop r15
    pop r14
    pop r13
    pop r12
    pop rbx
    pop rbp

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
