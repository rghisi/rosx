# Stage 1 boot sector (plans/bootloader phase 2).
# MBR-resident: bootstrap area only (bytes 0..446 — the packager splices
# this into the rxpart-encoded MBR, whose own table and signature own
# 446..512). Brings up COM1 and proves it; disk loading is phase 3.
# Linked with `ld -m elf_i386 -Ttext 0x7C00 --oformat binary`.

.code16

.equ COM1, 0x3F8

.text
.globl _start
_start:
    cli
    xorw %ax, %ax
    movw %ax, %ds
    movw %ax, %es
    movw %ax, %ss
    movw $0x7C00, %sp
    pushw %dx  # boot drive number, kept for the stage2 disk driver

    movb $0x00, COM1+1
    movb $0x80, COM1+3
    movb $0x01, COM1+0
    movb $0x00, COM1+1
    movb $0x03, COM1+3
    movb $0x0B, COM1+4
    movb $0xC7, COM1+2

    leaw msg_stage1_ok, %si
    call puts

halt_forever:
    hlt
    jmp halt_forever

puts:
    lodsb
    testb %al, %al
    jz puts_done
    call putc
    jmp puts
puts_done:
    ret

putc:
    pushw %ax
    movw $COM1+5, %dx
putc_wait:
    inb %dx, %al
    testb $0x20, %al
    jz putc_wait
    popw %ax
    movw $COM1, %dx
    outb %al, %dx
    ret

msg_stage1_ok:
    .asciz "[STAGE1] OK\r\n"

.org 510
.word 0xAA55
