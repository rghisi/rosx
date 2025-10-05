# --- Constants ---
.equ KERNEL_CS,        0x08       # Your Kernel Code Segment selector
.equ KERNEL_SS,        0x00       # Your Kernel Data/Stack Segment selector
.equ RFLAGS_INIT,      0x202      # Initial RFLAGS: Interrupts Enabled (IF=1)
.equ STACK_FRAME_SIZE, 144        # Total size of the frame in bytes (18 QWORDS)
                                  # 15 GPRs + 3 IRET frame fields (RIP, CS, RFLAGS only - same privilege)

# --- Stack Frame Layout (for restore_context) ---
# This layout matches what restore_context expects to pop from the stack.
# NOTE: Same-privilege iretq only uses RIP, CS, RFLAGS (no RSP/SS change)
#
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
# ------
# Total: 144 bytes (18 QWORDS)

.global initialize_process_stack
initialize_process_stack:
    # RDI: Pointer to the memory buffer for the new stack's TOP
    # RSI: The entry point address (RIP)
    # RDX: RFLAGS value to use

    # DEBUG: Print input parameters
    push rax
    push rbx
    push rcx
    push rdx
    push rdi
    push rsi

    mov al, 'I'
    out 0xe9, al
    mov al, 'N'
    out 0xe9, al
    mov al, ':'
    out 0xe9, al
    mov rax, rsi
    call print_hex_init
    mov al, 0x0a
    out 0xe9, al

    pop rsi
    pop rdi
    pop rdx
    pop rcx
    pop rbx
    pop rax

    # 1. Save original stack top and calculate frame base
    mov rcx, rdi                   # Save ORIGINAL stack top in RCX
    and rdi, ~0xF                  # Align RDI to 16 bytes
    sub rdi, STACK_FRAME_SIZE
    mov rax, rdi                   # RAX will be our return value (frame base)

    # 1b. Save the original stack top NOW before we clobber RCX
    # Store original stack top in CS field (offset 128)
    mov qword ptr [rax + 128], rcx

    # 2. Initialize all General Purpose Registers to 0

    # r15 - Offset 0
    mov qword ptr [rax + 0], 0x0

    # r14 - Offset 8
    mov qword ptr [rax + 8], 0x0

    # r13 - Offset 16
    mov qword ptr [rax + 16], 0x0

    # r12 - Offset 24
    mov qword ptr [rax + 24], 0x0

    # r11 - Offset 32
    mov qword ptr [rax + 32], 0x0

    # r10 - Offset 40
    mov qword ptr [rax + 40], 0x0

    # r9 - Offset 48
    mov qword ptr [rax + 48], 0x0

    # r8 - Offset 56
    mov qword ptr [rax + 56], 0x0

    # rdi - Offset 64
    mov qword ptr [rax + 64], 0x0

    # rsi - Offset 72
    mov qword ptr [rax + 72], 0x0

    # rbp - Offset 80
    mov qword ptr [rax + 80], 0x0

    # rdx - Offset 88
    mov qword ptr [rax + 88], 0x0

    # rcx - Offset 96
    mov qword ptr [rax + 96], 0x0

    # rbx - Offset 104
    mov qword ptr [rax + 104], 0x0

    # rax - Offset 112
    mov qword ptr [rax + 112], 0x0

    # 3. Build the IRET Stack Frame
    # For same-privilege iretq: only RIP, CS, RFLAGS (RSP stays unchanged)

    # RIP (Instruction Pointer) - Offset 120
    # DEBUG: Print address before writing
    push rax
    push rbx
    push rcx
    mov rbx, rax    # Save RAX FIRST
    mov al, 'W'
    out 0xe9, al
    mov al, 'R'
    out 0xe9, al
    mov al, 'A'
    out 0xe9, al
    mov al, ':'
    out 0xe9, al
    mov rax, rbx    # Restore RAX
    add rax, 120    # Calculate address
    call print_hex_init
    mov al, ' '
    out 0xe9, al
    mov al, 'V'
    out 0xe9, al
    mov al, ':'
    out 0xe9, al
    mov rax, rsi
    call print_hex_init
    mov al, 0x0a
    out 0xe9, al
    pop rcx
    pop rbx
    pop rax

    # DEBUG: Print RAX after restore
    push rax
    push rbx
    push rcx
    mov rbx, rax    # Save RAX in RBX FIRST before touching AL
    mov al, 'B'
    out 0xe9, al
    mov al, 'A'
    out 0xe9, al
    mov al, 'S'
    out 0xe9, al
    mov al, 'E'
    out 0xe9, al
    mov al, ':'
    out 0xe9, al
    mov rax, rbx    # Restore RAX from RBX for printing
    call print_hex_init
    mov al, 0x0a
    out 0xe9, al
    pop rcx
    pop rbx
    pop rax

    mov qword ptr [rax + 120], rsi

    # CS (Code Segment) - Offset 128
    # Already stored at the beginning (before RCX was clobbered)

    # RFLAGS - Offset 136
    # Use the RFLAGS value passed in RDX
    mov qword ptr [rax + 136], rdx

    # DEBUG: Print what we stored
    push rax
    push rbx
    push rcx
    push rdx

    mov rbx, rax    # Save RAX FIRST before touching AL!
    mov al, 'I'
    out 0xe9, al
    mov al, 'N'
    out 0xe9, al
    mov al, 'I'
    out 0xe9, al
    mov al, 'T'
    out 0xe9, al
    mov al, ' '
    out 0xe9, al
    mov al, 'R'
    out 0xe9, al
    mov al, 'I'
    out 0xe9, al
    mov al, 'P'
    out 0xe9, al
    mov al, ':'
    out 0xe9, al
    mov rax, [rbx + 120]
    call print_hex_init
    mov al, ' '
    out 0xe9, al
    mov al, 'S'
    out 0xe9, al
    mov al, 'T'
    out 0xe9, al
    mov al, 'K'
    out 0xe9, al
    mov al, ':'
    out 0xe9, al
    mov rax, [rbx + 128]
    call print_hex_init
    mov al, 0x0a
    out 0xe9, al

    pop rdx
    pop rcx
    pop rbx
    pop rax

    # 4. Return the new stack pointer (frame base in RAX)
    ret

# Helper function to print RAX in hex during initialization
# NOTE: This function DOES modify RAX (uses AL for output)
print_hex_init:
    push rbx
    push rcx
    push rdx
    push rax          # Save RAX since we modify AL

    mov rbx, rax
    mov rcx, 16

.print_loop_init:
    rol rbx, 4
    mov al, bl
    and al, 0x0f
    cmp al, 10
    jl .is_digit_init
    add al, 'a' - 10
    jmp .print_char_init
.is_digit_init:
    add al, '0'
.print_char_init:
    out 0xe9, al
    loop .print_loop_init

    pop rax           # Restore RAX
    pop rdx
    pop rcx
    pop rbx
    ret