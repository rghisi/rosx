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
    lgdt [gdt_pointer]
    mov eax, cr0
    or eax, {pe_bit}
    mov cr0, eax
    jmp fword ptr [pm_entry_pointer]

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
