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

.equ DEBUG_PORT, 0xE9
.equ DEBUG_EXIT_PORT, 0xF4

.text
.globl _start
_start:
    cli
    xorw %ax, %ax
    movw %ax, %ds
    movw %ax, %ss
    movw $0x7C00, %sp

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

# putc: emit AL on the debug port, clobbering only DX.
putc:
    movw $DEBUG_PORT, %dx
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
