use rosx_boot_common::gdt;

core::arch::global_asm!(
    r#"
    .section .text.entry, "ax", @progbits
    .code16
    .globl _start
_start:
    cli
    xor ax, ax
    mov ds, ax
    mov es, ax
    call enable_a20
    lgdt [gdt_pointer]
    mov eax, cr0
    or eax, {pe_bit}
    mov cr0, eax
    jmp fword ptr [pm_entry_pointer]

enable_a20:
    pusha
    mov ax, 0x2401
    int 0x15
    in al, 0x92
    or al, 0x02
    out 0x92, al
    call a20_8042
    call a20_probe
    popa
    ret

a20_8042:
    mov cx, 0xFFFF
a20_8042_wait_ready:
    in al, 0x64
    test al, 0x02
    jz a20_8042_read_command
    loop a20_8042_wait_ready
    ret
a20_8042_read_command:
    mov al, 0xD0
    out 0x64, al
    mov cx, 0xFFFF
a20_8042_wait_data:
    in al, 0x64
    test al, 0x01
    jnz a20_8042_read_data
    loop a20_8042_wait_data
    ret
a20_8042_read_data:
    in al, 0x60
    or al, 0x02
    mov ah, al
    mov cx, 0xFFFF
a20_8042_wait_ready2:
    in al, 0x64
    test al, 0x02
    jz a20_8042_write_command
    loop a20_8042_wait_ready2
    ret
a20_8042_write_command:
    mov al, 0xD1
    out 0x64, al
    mov cx, 0xFFFF
a20_8042_wait_ready3:
    in al, 0x64
    test al, 0x02
    jz a20_8042_write_data
    loop a20_8042_wait_ready3
    ret
a20_8042_write_data:
    mov al, ah
    out 0x60, al
    ret

a20_probe:
    mov ax, [0x0500]
    mov [a20_saved_low], ax
    mov ax, 0xFFFF
    mov es, ax
    mov ax, word ptr es:[0x0510]
    mov [a20_saved_high], ax
    mov word ptr [0x0500], 0xC0DE
    mov word ptr es:[0x0510], 0xBEEF
    cmp word ptr [0x0500], 0xC0DE
    jne a20_probe_failed
    mov ax, [a20_saved_low]
    mov [0x0500], ax
    mov ax, [a20_saved_high]
    mov word ptr es:[0x0510], ax
    ret

a20_probe_failed:
    mov si, offset msg_err_a20
    call puts16
    mov al, 0x01
    out 0xF4, al
1:  hlt
    jmp 1b

puts16:
    lodsb
    test al, al
    jz puts16_done
    call putc16
    jmp puts16
puts16_done:
    ret

putc16:
    push ax
    mov dx, 0x3FD
putc16_wait:
    in al, dx
    test al, 0x20
    jz putc16_wait
    pop ax
    mov dx, 0x3F8
    out dx, al
    ret

    .code32
protected_entry:
    mov ax, {data_selector}
    mov ds, ax
    mov es, ax
    mov fs, ax
    mov gs, ax
    mov ss, ax
    mov esp, {pm_stack_top}
    mov edi, offset _bss_start
    mov ecx, offset _bss_end
    sub ecx, edi
    xor eax, eax
    rep stosb
    call {stage2_main}
1:  hlt
    jmp 1b

    .section .rodata
    .p2align 2
gdt_pointer:
    .short {gdt_limit}
    .long {gdt_table}
pm_entry_pointer:
    .long protected_entry
    .short {code32_selector}
msg_err_a20:
    .asciz "[ERR] a20\r\n"

    .section .bss
a20_saved_low:
    .skip 2
a20_saved_high:
    .skip 2
    "#,
    pe_bit = const 1u32,
    code32_selector = const 0x08u32,
    data_selector = const 0x10u32,
    pm_stack_top = const 0x0007_F000u32,
    gdt_limit = const (core::mem::size_of::<[u64; 5]>() - 1),
    gdt_table = sym GDT,
    stage2_main = sym crate::stage2_main,
);

#[used]
static GDT: [u64; 5] = [
    gdt::NULL,
    gdt::code32(),
    gdt::data32(),
    gdt::code16(),
    gdt::data16(),
];
