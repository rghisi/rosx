# Stage 2 entry stub (plans/bootloader phase 3).
# Loaded by stage1 at 0x0000:0x7E00 and reached with `ljmp $0,$0x7E00`; DL
# (the boot drive) is still live on entry. This is a throwaway stand-in for
# the real stage2 crate that replaces it in phase 4, so it carries only the
# [STAGE2_16] checkpoint: bring up COM1, print the marker, take the QEMU
# debug exit (0x10 -> exit 33), halt. It is NOT a boot sector: linked with
# `ld -m elf_i386 -Ttext 0x7E00 --oformat binary`, no 512-byte gate, no
# boot signature, size budget 160 KiB enforced by the packager.

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

    movb $0x00, COM1+1        # IER = 0
    movb $0x80, COM1+3        # LCR: DLAB = 1
    movb $0x01, COM1+0        # divisor low  = 1  (115200)
    movb $0x00, COM1+1        # divisor high = 0
    movb $0x03, COM1+3        # LCR: 8N1, DLAB = 0
    movb $0x0B, COM1+4        # FCR: enable + reset FIFO
    movb $0xC7, COM1+2        # MCR: DTR | RTS | OUT2

    leaw msg_stage2_ok, %si
    call puts

    movb $0x10, %al           # success verdict -> QEMU exit (0x10<<1)|1 = 33
    outb %al, $DEBUG_EXIT_PORT

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

msg_stage2_ok:
    .asciz "[STAGE2_16] OK\r\n"
