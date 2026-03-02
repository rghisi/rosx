/* Multiboot v1 header — must be within first 8 KB and 4-byte aligned */
.section .multiboot, "a"
.align 4
multiboot_header:
    .long 0x1BADB002                         /* magic */
    .long 0x00000000                         /* flags */
    .long -(0x1BADB002 + 0x00000000)         /* checksum */

/* 64 KB boot stack in BSS (zero-initialised, not included in the binary image) */
.section .bss
.align 16
boot_stack_bottom:
    .skip 65536
boot_stack_top:

/* Entry point — bootloader leaves eax = 0x2BADB002, ebx = multiboot info pointer */
.section .text
.global _start
_start:
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
