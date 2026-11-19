# Stage 1 boot sector (plans/bootloader phases 2-3).
# MBR-resident: code+data live in bytes 0..445 (the packager splices this
# into the rxpart-encoded MBR; its own table and signature own 446..512).
# Prints on two channels — port 0xE9 (the emulator verdict stream) and the
# BIOS teletype on the VGA console (plans/bootloader 13-console.md) — loads
# the [ROS2 header][stage2 bin] image at LBA 8 into
# 0x0000:0x7E00 via the INT 13h LBA extensions, falling back to CHS when the
# BIOS lacks them (super-floppy media), then jumps. Linked with
# `ld -m elf_i386 -Ttext 0x7C00 --oformat binary`. The packager assembles with
# --defsym HARD_UNIT=1 for the MBR splice (a hard boot may hand a bogus DL) and
# HARD_UNIT=0 for the floppy image (DL is the real floppy unit).
#
# Budget notes: the image body starts at LBA 9 and is <= 321 sectors, so every
# LBA this stage handles fits in 16 bits; cur_lba/cur_rem and the CHS math are
# 16-bit throughout (native in .code16, no 0x66 prefixes). Failed reads reset
# the controller and re-issue: era firmware (Phoenix 4.04 on UMC 8881) hands
# the first INT 13h reads after boot a timeout (AH=80h) that QEMU never makes.

.code16

.equ DEBUG_PORT, 0xE9
.equ HDR_SCRATCH, 0x900          # header sector lands here to be validated
.equ LOAD_OFF, 0x7E00            # stage2 image destination (physical)
.equ STAGE2_TOTAL_MAX, 164352    # 160 KiB + 512 (the 0x7E00..0x2FFFF budget)

.text
.globl _start
_start:
    cli
    xorw %ax, %ax
    movw %ax, %ds
    movw %ax, %es
    movw %ax, %ss
    movw $0x7C00, %sp
.if HARD_UNIT
    orb $0x80, %dl               # Phoenix 4.04 on UMC 8881 hands the MBR DL=00;
.endif                           # the running disk is a hard unit (era idiom)
    movb %dl, boot_drive         # BIOS boot drive, needed by INT 13h and stage2

    leaw msg_stage1_ok, %si
    call puts

    call load_stage2             # returns only on success

    movb boot_drive, %dl         # stage2 inherits the boot drive in DL
    ljmp $0x0000, $LOAD_OFF      # never returns

halt_forever:
    hlt
    jmp halt_forever

# load_stage2: probe the LBA extensions (CHS fallback otherwise), read the
# header into HDR_SCRATCH, validate it, then load the image body at LOAD_OFF.
load_stage2:
    movb $0x41, %ah
    movw $0x55AA, %bx
    movb boot_drive, %dl
    int $0x13
    movb $0x01, use_lba
    jc chs_geom
    cmpw $0xAA55, %bx            # the canonical EDD marker (AH is a version byte)
    je read_image
chs_geom:
    movb $0x00, use_lba
    movb $0x08, %ah              # geometry: CL low 6 = sectors/track, DH = max head
    movb boot_drive, %dl
    movw $0x0500, %di            # parameter table to scratch, never the IVT
    int $0x13
    movb %cl, chs_sptrack
    andb $0x3F, chs_sptrack
    incb %dh                     # head count = max head + 1
    movb %dh, chs_heads

read_image:
    movw $8, cur_lba             # header sector
    movw $(HDR_SCRATCH >> 4), buf_seg
    movw $0x0001, cur_rem
    call read_sectors
    call validate_header         # leaves the body sector count in cur_rem
    movw $9, cur_lba             # body follows the one-sector header
    movw $(LOAD_OFF >> 4), buf_seg
    call read_sectors
    ret

# read_sectors: transfer cur_rem sectors at cur_lba into buf_seg:0, picking the
# LBA or CHS driver from use_lba. A failed read resets the controller and
# re-issues (era timeout discipline); the CHS loop re-arms the count per
# sector. A timing-out LBA face hands the whole load to the CHS geometry path
# once before the error becomes fatal.
read_sectors:
    movb $3, retries
drive_once:
    cmpb $0x00, use_lba
    je chs_read
    movw cur_rem, %ax
    movw %ax, dap_count
    movw buf_seg, %ax
    movw %ax, dap_seg            # 512-byte steps keep the DAP offset at zero
    mov cur_lba, %ax
    movw %ax, dap_lba_lo         # the upper six LBA bytes are zero-init, never rewritten
    movb $0x42, %ah
    movb boot_drive, %dl
    leaw dap, %si
    int $0x13
    jc reset_retry
    ret

reset_retry:
    decb retries
    jz lba_or_die
    movb $0x00, %ah              # reset the controller, as era DOS did
    movb boot_drive, %dl
    int $0x13
    jmp drive_once
lba_or_die:
    cmpb $0x00, use_lba
    jne chs_geom                 # a faking EDD face: restart the load over CHS, once
    jmp disk_err

# chs_read: read cur_rem sectors from cur_lba into buf_seg:0, one per INT 13h.
chs_read:
    cmpw $0, cur_rem
    je chs_done
    movb chs_heads, %al
    mulb chs_sptrack             # AX = heads * sectors-per-track
    movw %ax, %bx
    mov cur_lba, %ax
    xorw %dx, %dx
    divw %bx                     # AX = cylinder, DX = within-cylinder offset
    movw %ax, %bx                # BX = cylinder (sectors-per-cyl now dead)
    movw %dx, %ax
    divb chs_sptrack             # AL = head, AH = zero-based sector

    movb %al, %dh                # head
    incb %ah                     # CHS sectors are 1-based
    movb %bl, %ch                # cylinder bits 7:0
    movb %bh, %al                # cylinder bits 15:8
    andb $0x03, %al              # keep bits 9:8
    shlb $6, %al                 # into CL bits 7:6
    orb %ah, %al
    movb %al, %cl
    movb boot_drive, %dl
    movw buf_seg, %ax
    movw %ax, %es                # ES:0 reaches past the 64 KiB offset ceiling
    xorw %bx, %bx
    movb $0x01, %al              # one sector
    movb $0x02, %ah
    int $0x13
    jc reset_retry

    addw $0x20, buf_seg          # next 512-byte paragraph
    incw cur_lba
    decw cur_rem
    movb $3, retries             # the allowance is per sector: fresh on every advance
    jmp chs_read
chs_done:
    ret

# validate_header: check the ROS2 magic and size bound at HDR_SCRATCH and leave
# the body sector count in cur_rem. A bad header is a fatal disk error.
validate_header:
    cmpl $0x32534F52, HDR_SCRATCH   # 'R''O''S''2' little-endian
    jne disk_err
    movl HDR_SCRATCH+8, %eax        # total_bytes_including_header
    cmpl $STAGE2_TOTAL_MAX, %eax
    ja disk_err
    shrl $9, %eax                   # whole-image sector count (totals are sector multiples)
    decl %eax                       # minus the header sector
    jz disk_err                     # no body to load
    movw %ax, cur_rem
    ret

# disk_err: report the failing LBA (low byte, hex) and halt (no debug-exit yet).
disk_err:
    leaw msg_err_disk, %si
    call puts
    call put_hex
    jmp halt_forever

puts:
    lodsb
    testb %al, %al
    jz puts_done
    call putc
    jmp puts
puts_done:
    ret

# put_hex: print the low byte of cur_lba as two uppercase hex digits.
put_hex:
    mov cur_lba, %al
    movb %al, %ah
    shrb $4, %al
    call emit_nybble
    movb %ah, %al
    call emit_nybble
    ret
emit_nybble:
    andb $0x0F, %al
    addb $0x30, %al
    cmpb $0x39, %al
    jbe emit_num
    addb $7, %al
emit_num:
    call putc
    ret

# putc: emit AL on both channels — port 0xE9 carries the test verdict
# stream (an architecturally harmless no-op on silicon) and the BIOS
# teletype carries the human VGA stream, BIOS-owned cursor and all. AX is
# preserved around the INT: put_hex keeps its second nybble in ah, and INT
# 10h is permitted to destroy AX. DX is clobbered by both channels.
putc:
    pushw %ax
    movw $DEBUG_PORT, %dx
    outb %al, %dx
    movb $0x0E, %ah              # teletype output, AL=char
    xorb %bh, %bh                # page 0
    int $0x10
    popw %ax
    ret

msg_stage1_ok:
    .asciz "[STAGE1] OK\r\n"
msg_err_disk:
    .asciz "[ERR] disk "

boot_drive:
    .byte 0
chs_sptrack:
    .byte 0
chs_heads:
    .byte 0
use_lba:
    .byte 0
retries:
    .byte 0
dap:
dap_size:
    .byte 0x10
dap_resv:
    .byte 0
dap_count:
    .byte 0, 0
dap_off:
    .byte 0, 0                   # stays zero: every transfer is paragraph-aligned
dap_seg:
    .byte 0, 0
dap_lba_lo:
    .byte 0, 0
    .byte 0, 0, 0, 0, 0, 0
buf_seg:
    .word 0
cur_lba:
    .word 0
cur_rem:
    .word 0

.org 510
.word 0xAA55
