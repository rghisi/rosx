/* Multiboot v1 header — must be within first 8 KB and 4-byte aligned */
.section .multiboot, "a"
.align 4
multiboot_header:
    .long 0x1BADB002                         /* magic */
    .long 0x00000000                         /* flags */
    .long -(0x1BADB002 + 0x00000000)         /* checksum */

/* Own descriptor table: the multiboot spec guarantees a flat memory model
   but not the layout of the loader's GDT (SeaBIOS puts code at 0x08, GRUB
   at 0x10 with GDT[1] NULL).  Install flat selectors so the IDT's hardcoded
   0x08 is true for every loader. */
.section .rodata
.align 8
gdt_start:
    .quad 0x0000000000000000                 /* 0x00: null descriptor */
gdt_code:
    .quad 0x00CF9A000000FFFF                 /* 0x08: code, base 0, limit 4G, G|D|P */
gdt_data:
    .quad 0x00CF92000000FFFF                 /* 0x10: data, base 0, limit 4G, G|D|P */
gdt_end:

gdt_descriptor:
    .word gdt_end - gdt_start - 1
    .long gdt_start

/* 64 KB boot stack in BSS (zero-initialised, not included in the binary image) */
.section .bss
.align 16
boot_stack_bottom:
    .skip 65536
boot_stack_top:

/* Entry point — bootloader leaves eax = 0x2BADB002, ebx = multiboot info pointer.
   eax and ebx carry the kernel_main arguments, so every scratch register
   below is ecx. */
.section .text
.global _start
_start:
    cli
    lgdt [gdt_descriptor]
    push 0x08                                /* far-reload CS with our code selector; */
    lea ecx, [.after_gdt]                    /* LLVM Intel syntax has no label far-jmp */
    push ecx
    retf
.after_gdt:
    mov ecx, 0x10                            /* our data selector */
    mov ds, cx
    mov es, cx
    mov fs, cx
    mov gs, cx
    mov ss, cx

    /* Load stack pointer; align to 16 bytes per the System V i386 ABI */
    lea esp, [boot_stack_top]
    and esp, -16

    /* Adjust so that esp is 16-byte aligned just before the call instruction.
       We push two 4-byte arguments (8 bytes total), so sub 8 here means the
       stack is 16-byte aligned once both args are on it. */
    sub esp, 8
    push ebx    /* arg2: multiboot info pointer */
    push eax    /* arg1: multiboot magic         */
    call kernel_main

.hang:
    hlt
    jmp .hang
