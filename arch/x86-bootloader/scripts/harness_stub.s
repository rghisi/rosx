# Harness self-test stub (plans/bootloader phase 1, checkpoint 2).
# One source, four behaviors selected at assembly time with
# `as --32 --defsym VARIANT=...`:
#   0 = pass: print banner + OK, write 0x10 to 0xF4 (QEMU exit 33)
#   1 = fail: print banner + FAIL, write 0x01 to 0xF4 (QEMU exit 3)
#   2 = crash: zeroed IDT limit + INT 3 -> deterministic triple fault
#       (measured QEMU exit 0 under -no-reboot)
#   3 = hang: print banner and halt forever (harness timeout verdict)
# Linked with `ld -m elf_i386 -Ttext 0x7C00 --oformat binary`: file offset
# 0 is execution start (CS:IP 0:7C00) and offset 510 carries the signature.

.code16

.equ COM1, 0x3F8
.equ DEBUG_EXIT_PORT, 0xF4

.text
.globl _start
_start:
    cli
    xorw %ax, %ax
    movw %ax, %ds
    movw %ax, %ss
    movw $0x7C00, %sp

    movb $0x00, COM1+1
    movb $0x80, COM1+3
    movb $0x01, COM1+0
    movb $0x00, COM1+1
    movb $0x03, COM1+3
    movb $0x0B, COM1+4
    movb $0xC7, COM1+2

    leaw banner, %si
    call puts

.if VARIANT == 0
    leaw msg_ok, %si
    call puts
    movb $0x10, %al
    outb %al, $DEBUG_EXIT_PORT
.endif
.if VARIANT == 1
    leaw msg_fail, %si
    call puts
    movb $0x01, %al
    outb %al, $DEBUG_EXIT_PORT
.endif
.if VARIANT == 2
    lidt bogus_idtr
    int $3
.endif
.if VARIANT == 3
.endif

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

banner:
    .asciz "BOOTLOADER-HARNESS "
msg_ok:
    .asciz "OK\r\n"
msg_fail:
    .asciz "FAIL\r\n"
bogus_idtr:
    .word 0x0000
    .long 0x00000000

.org 510
.word 0xAA55
