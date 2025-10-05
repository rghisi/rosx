# --- Constants ---
.equ KERNEL_CS,        0x08       # Your Kernel Code Segment selector
.equ KERNEL_SS,        0x00       # Your Kernel Data Segment selector
.equ RFLAGS_INIT,      0x202      # Initial RFLAGS: Interrupts Enabled (IF=1)
.equ STACK_FRAME_SIZE, 56         # Total size of the frame in bytes (7 QWORDS)

.global initialize_process_stack
initialize_process_stack:
    # RDI: Pointer to the memory buffer for the new stack's TOP
    # RSI: The entry point address (RIP)

    # 1. Reserve space for the context frame
    sub rdi, STACK_FRAME_SIZE
    mov rsp, rdi                   # Temporarily use RDI as the stack pointer

    # 2. Build the Interrupt Stack Frame

    # RBP (Base Pointer) or Padding - Offset 0
    mov qword ptr [rsp + 0], 0x0

    # RIP (Instruction Pointer) - Offset 8
    mov qword ptr [rsp + 8], rsi

    # CS (Code Segment) - Offset 16
    mov qword ptr [rsp + 16], KERNEL_CS

    # RFLAGS - Offset 24
    mov qword ptr [rsp + 24], RFLAGS_INIT

    # RSP (Stack Pointer) - Offset 32
    # Value is the address just after the iretq frame is popped.
    add rdi, STACK_FRAME_SIZE
    mov qword ptr [rsp + 32], rdi
    sub rdi, STACK_FRAME_SIZE       # Restore the temporary stack pointer

    # SS (Stack Segment) - Offset 40
    mov qword ptr [rsp + 40], KERNEL_SS

    # Padding/Last GPR slot - Offset 48
    mov qword ptr [rsp + 48], 0x0

    # 3. Return the new stack pointer
    mov rax, rdi
    ret