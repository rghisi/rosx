/* Own descriptor table: the chain enters long mode with stage2's GDT, whose
   image span (0x7E00..0x2FFFF) is published BootloaderReclaimable, so the
   kernel installs the same frozen layout here (plans/bootloader/00-boot-abi.md
   §4) — the selectors STAR hardcodes in cpu.rs must not change.
   The kernel is linked in the +2 TiB window (0xFFFF800000000000), outside
   every 32-bit addressing form, so all data references here are RIP-relative
   or full 64-bit quads, and the target uses the large code model. */
.section .rodata
.align 8
gdt_start:
    .quad 0x0000000000000000                 /* 0x00: null descriptor */
    .quad 0x00AF9A000000FFFF                 /* 0x08: code64, base 0, limit 4G, L=1 G|P */
    .quad 0x00CF92000000FFFF                 /* 0x10: data,   base 0, limit 4G, D=1 G|P */
    .quad 0x00CFF2000000FFFF                 /* 0x18: user data (ring 3), same geometry */
    .quad 0x00AFFA000000FFFF                 /* 0x20: user code64 (ring 3), same geometry */
gdt_end:

/* GDTR operand: 16-bit limit followed by the 64-bit base at offset 2 (an
   lgdt operand needs no alignment). */
gdt_descriptor:
    .short gdt_end - gdt_start - 1
    .quad gdt_start

/* 64 KB boot stack in BSS (zero-initialised, not included in the binary image) */
.section .bss
.align 16
boot_stack_bottom:
    .skip 65536
boot_stack_top:

/* Entry point — stage2-64 jmps here with rax = BOOT_INFO_MAGIC,
   rdi = &BootInfo (higher half), rsp 16-byte aligned in the higher half,
   CS = 0x08, DS = ES = SS = 0x10, IF = 0. kernel_main takes the magic in rdi
   and the boot info pointer in rsi (System V AMD64); the far reload and the
   data-selector loads consume rax, so the incoming pair is parked in the
   volatile r10/r11 first and rax is the only other scratch below. */
.section .text
.global _start
_start:
    cli
    mov r10, rax                             /* park boot magic  */
    mov r11, rdi                             /* park boot info   */
    lgdt fword ptr [rip + gdt_descriptor]
    push 0x08                                /* far-reload CS with our code64 selector; */
    lea rax, [rip + after_gdt]               /* LLVM Intel syntax has no label far-jmp */
    push rax
    retfq
after_gdt:
    mov ecx, 0x10                            /* our data selector */
    mov ds, cx
    mov es, cx
    mov fs, cx
    mov gs, cx
    mov ss, cx

    lea rsp, [rip + boot_stack_top]          /* kernel-owned boot stack */
    xor ebp, ebp                             /* mark the outermost frame (System V AMD64) */
    and rsp, -16                             /* 16-byte align the stack before the call */

    mov rdi, r10                             /* arg1: boot magic        */
    mov rsi, r11                             /* arg2: boot info pointer */
    call kernel_main

.hang:
    cli
    hlt
    jmp .hang
