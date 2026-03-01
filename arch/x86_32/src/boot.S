/* Multiboot2 header — must be within first 32 KB and 8-byte aligned */
.section .multiboot, "a"
.align 8
multiboot_header:
    .long 0xE85250D6                                             /* magic */
    .long 0                                                      /* arch: i386 protected mode */
    .long (multiboot_header_end - multiboot_header)              /* header length */
    .long -(0xE85250D6 + 0 + (multiboot_header_end - multiboot_header)) /* checksum */
    /* end tag */
    .short 0    /* type  */
    .short 0    /* flags */
    .long  8    /* size  */
multiboot_header_end:

/* 64 KB boot stack in BSS (zero-initialised, not included in the binary image) */
.section .bss
.align 16
boot_stack_bottom:
    .skip 65536
boot_stack_top:

/* Entry point — GRUB leaves eax = Multiboot2 magic, ebx = info-struct pointer */
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
