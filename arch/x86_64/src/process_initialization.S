# --- Constants ---
.equ KERNEL_CS,        0x08       # Your Kernel Code Segment selector
.equ KERNEL_SS,        0x00       # Your Kernel Data/Stack Segment selector
.equ RFLAGS_INIT,      0x202      # Initial RFLAGS: Interrupts Enabled (IF=1)
.equ STACK_FRAME_SIZE, 144        # Total size of the frame in bytes (18 QWORDS)
                                  # 15 GPRs + 3 IRET frame fields (RIP, CS, RFLAGS only - same privilege)

.global initialize_task_for_interrupt

# ============================================================================
# initialize_task_for_interrupt: Set up task stack for IRETQ-based entry
# ============================================================================
# Creates a fake interrupt frame so that swap_context (which now uses IRETQ)
# can start the task for the first time.
#
# Parameters:
#   RDI: Stack top pointer
#   RSI: Entry point address (goes into RIP - where execution will start)
#   RDX: Parameter to pass to entry point (goes into RDI register)
#
# Returns:
#   RAX: New stack pointer (pointing to the base of the interrupt frame)
#
# Stack layout after initialization (compatible with IRETQ):
#   [RSP + 0]:   r15 (0)
#   [RSP + 8]:   r14 (0)
#   [RSP + 16]:  r13 (0)
#   [RSP + 24]:  r12 (0)
#   [RSP + 32]:  r11 (0)
#   [RSP + 40]:  r10 (0)
#   [RSP + 48]:  r9 (0)
#   [RSP + 56]:  r8 (0)
#   [RSP + 64]:  rdi (RDX parameter - passed as first argument)
#   [RSP + 72]:  rsi (0)
#   [RSP + 80]:  rbp (0)
#   [RSP + 88]:  rdx (0)
#   [RSP + 96]:  rcx (0)
#   [RSP + 104]: rbx (0)
#   [RSP + 112]: rax (0)
#   [RSP + 120]: RIP (RSI parameter - entry point address)
#   [RSP + 128]: CS (kernel code segment)
#   [RSP + 136]: RFLAGS (0x202 - interrupts enabled)
#
# When swap_context is called with this stack, it will:
#   1. Pop all 15 GPRs (RDI will contain the RDX parameter)
#   2. Execute IRETQ which pops RIP, CS, RFLAGS and jumps to RIP (RSI parameter)
#
initialize_task_for_interrupt:
    # Save parameters
    # RSI = entry point (goes in RIP)
    # RDX = parameter (goes in RDI)
    mov r8, rsi   # r8 = entry point address (will go in RIP)
    mov r9, rdx   # r9 = entry_param (will go in RDI register)

    # Align stack to 16 bytes and reserve space for interrupt frame
    mov rax, rdi
    and rax, ~0xF                  # Align to 16-byte boundary
    sub rax, STACK_FRAME_SIZE      # Reserve 144 bytes (15 GPRs + RIP + CS + RFLAGS)

    # RAX now points to the base of our interrupt frame

    # ========================================================================
    # Initialize all 15 General Purpose Registers
    # ========================================================================
    # Most registers start at 0, except RDI which gets the entry parameter

    mov qword ptr [rax + 0], 0     # r15
    mov qword ptr [rax + 8], 0     # r14
    mov qword ptr [rax + 16], 0    # r13
    mov qword ptr [rax + 24], 0    # r12
    mov qword ptr [rax + 32], 0    # r11
    mov qword ptr [rax + 40], 0    # r10
    mov qword ptr [rax + 48], 0    # r9
    mov qword ptr [rax + 56], 0    # r8
    mov qword ptr [rax + 64], r9   # rdi = entry_param (first argument to task)
    mov qword ptr [rax + 72], 0    # rsi
    mov qword ptr [rax + 80], 0    # rbp
    mov qword ptr [rax + 88], 0    # rdx
    mov qword ptr [rax + 96], 0    # rcx
    mov qword ptr [rax + 104], 0   # rbx
    mov qword ptr [rax + 112], 0   # rax

    # ========================================================================
    # Build the IRET Frame (what the CPU pushes on interrupt)
    # ========================================================================

    # RIP - Instruction Pointer (where IRETQ will jump to)
    mov qword ptr [rax + 120], r8   # RIP = entry point address

    # CS - Code Segment (need current kernel CS)
    mov rbx, 0
    mov bx, cs                      # Read current CS selector
    mov qword ptr [rax + 128], rbx  # Store CS value

    # RFLAGS - CPU flags (enable interrupts)
    mov qword ptr [rax + 136], RFLAGS_INIT  # RFLAGS = 0x202 (IF=1, bit 1 set)

    # Return the stack pointer (points to r15 at the base of the frame)
    ret