# --- Constants ---
.equ KERNEL_CS,        0x08       # Your Kernel Code Segment selector
.equ KERNEL_SS,        0x00       # Your Kernel Data/Stack Segment selector
.equ RFLAGS_INIT,      0x202      # Initial RFLAGS: Interrupts Enabled (IF=1)
.global initialize_task_stack

initialize_task_stack:
    # Save parameters
    # RDI = stack pointer to initialize
    # RSI = entry point to call
    # RDX = parameter to provide the entry point with

    push 0x00       #r15
    push 0x00       #r14
    push 0x00       #r13
    push 0x00       #r12
    push 0x00       #r11
    push 0x00       #r10
    push 0x00       #r9
    push 0x00       #r8
    push rdi        #rdi
    push rsi        #rsi
    push 0x00       #rbp
    push rdx        #rdx
    push 0x00       #rcx
    push 0x00       #rbx
    push 0x00       #rax

    ret