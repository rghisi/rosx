.global syscall_handler_entry

# Parameters from 'syscall' instruction:
# RAX = Syscall Number
# RDI, RSI, RDX = Arguments
# RCX = Return RIP (saved by CPU)
# R11 = Saved RFLAGS (saved by CPU)

syscall_handler_entry:
    # Save registers
    push r15
    push r14
    push r13
    push r12
    push r11 # Saved RFLAGS
    push rcx # Saved RIP
    push rbp
    push rbx

    # Prepare arguments for Rust syscall_handler(num, arg1, arg2, arg3)
    # RDI = num (RAX)
    # RSI = arg1 (RDI)
    # RDX = arg2 (RSI)
    # RCX = arg3 (RDX)
    mov rcx, rdx
    mov rdx, rsi
    mov rsi, rdi
    mov rdi, rax

    call syscall_handler # result in RAX

    pop rbx
    pop rbp
    pop rcx
    pop r11
    pop r12
    pop r13
    pop r14
    pop r15

    # Return to Ring 0 (since we don't support Ring 3 yet)
    # Restore RFLAGS from R11
    push r11
    popfq

    # Jump to return address in RCX
    jmp rcx