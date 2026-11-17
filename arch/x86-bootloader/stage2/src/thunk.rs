use rosx_boot_common::bios::Regs16;
use rosx_boot_common::bios::Regs32;
use rosx_boot_common::bios::TRAMPOLINE_BASE;
use rosx_boot_common::bios::TRAMPOLINE_BUDGET;
use rosx_boot_common::bios::checksum;
use rosx_boot_common::bios::region_checksum;
use rosx_boot_common::bios::stable_region_checksum;

const M_FAR_PM16_IN: u32 = TRAMPOLINE_BASE as u32 + 0x120;
const M_FAR_RM: u32 = TRAMPOLINE_BASE as u32 + 0x126;
const M_FAR_RM_WIDE: u32 = TRAMPOLINE_BASE as u32 + 0x12C;
const M_FAR_PM16_OUT: u32 = TRAMPOLINE_BASE as u32 + 0x132;
const M_FAR_PM32_OUT: u32 = TRAMPOLINE_BASE as u32 + 0x138;
const M_REGS: u32 = TRAMPOLINE_BASE as u32 + 0x148;
const M_WIDE: u32 = TRAMPOLINE_BASE as u32 + 0x160;
const M_SAVED_ESP: u32 = TRAMPOLINE_BASE as u32 + 0x180;
const M_REGS_PTR: u32 = TRAMPOLINE_BASE as u32 + 0x184;
const M_INT_NO: u32 = TRAMPOLINE_BASE as u32 + 0x188;
const M_SAVED_EFLAGS: u32 = TRAMPOLINE_BASE as u32 + 0x18C;
const M_RET_SRC: u32 = TRAMPOLINE_BASE as u32 + 0x190;
const M_RET_CNT: u32 = TRAMPOLINE_BASE as u32 + 0x194;
const _: () = assert!(TRAMPOLINE_BASE + TRAMPOLINE_BUDGET <= 0xFFFF);
const RM_STACK_TOP: u32 = 0x7A00;
const PE_BIT: u32 = 1;
const PE_CLEAR_MASK: u32 = 0x7FFF_FFFE;
const WIDE_MARK: u32 = 0x80;

unsafe extern "C" {
    fn thunk_entry32();
    fn thunk_entry32_wide();
    static thunk_blob_start: u8;
    static thunk_blob_end: u8;
}

pub struct BiosThunk {
    stable_checksum: u16,
}

impl BiosThunk {
    pub fn install() -> Option<Self> {
        let start = &raw const thunk_blob_start;
        let end = &raw const thunk_blob_end;
        if (end as usize) - (start as usize) != TRAMPOLINE_BUDGET {
            return None;
        }
        unsafe {
            core::ptr::copy_nonoverlapping(start, TRAMPOLINE_BASE as *mut u8, TRAMPOLINE_BUDGET);
            if checksum(core::slice::from_raw_parts(start, TRAMPOLINE_BUDGET)) != region_checksum()
            {
                return None;
            }
            Some(Self {
                stable_checksum: stable_region_checksum(),
            })
        }
    }

    pub fn call(&self, int_no: u8, regs: &mut Regs16) {
        let regs_address = regs as *mut Regs16 as usize;
        unsafe {
            core::arch::asm!(
                "call {entry}",
                entry = sym thunk_entry32,
                inlateout("eax") u32::from(int_no) => _,
                inlateout("edx") regs_address => _,
                lateout("ecx") _,
                options(preserves_flags),
            );
        }
    }

    pub fn call32(&self, regs: &mut Regs32) {
        let regs_address = regs as *mut Regs32 as usize;
        unsafe {
            core::arch::asm!(
                "call {entry}",
                entry = sym thunk_entry32_wide,
                inlateout("edx") regs_address => _,
                lateout("eax") _,
                lateout("ecx") _,
                options(preserves_flags),
            );
        }
    }

    pub fn intact(&self) -> bool {
        unsafe { stable_region_checksum() == self.stable_checksum }
    }
}

core::arch::global_asm!(
    r#"
    .section .text.thunk_blob, "ax", @progbits
    .code16
    .globl thunk_blob_start
thunk_blob_start:
pm16_landing:
    mov eax, cr0
    and eax, {pe_clear_mask}
    mov cr0, eax
    xor ax, ax
    mov ds, ax
    mov es, ax
    mov fs, ax
    mov gs, ax
    mov ss, ax
    mov sp, {rm_stack}
    test byte ptr [{int_no}], {wide_mark}
    js wide_far
    jmp fword ptr [{far_rm}]
wide_far:
    jmp fword ptr [{far_rm_wide}]

    .code16
rm_entry:
    cli
    mov ax, [{regs}]
    mov bx, [{regs}+2]
    mov cx, [{regs}+4]
    mov dx, [{regs}+6]
    mov si, [{regs}+8]
    mov di, [{regs}+10]
    mov bp, [{regs}+12]
    cmp byte ptr [{int_no}], 0x10
    je thunk_int10
    cmp byte ptr [{int_no}], 0x13
    je thunk_int13
    cmp byte ptr [{int_no}], 0x15
    je thunk_int15
    jmp store_results
thunk_int10:
    int 0x10
    jmp store_results
thunk_int13:
    int 0x13
    jmp store_results
thunk_int15:
    int 0x15
store_results:
    push eax
    pushfd
    pop eax
    mov [{regs}+14], ax
    pop eax
    mov [{regs}], ax
    mov [{regs}+2], bx
    mov [{regs}+4], cx
    mov [{regs}+6], dx
    mov [{regs}+8], si
    mov [{regs}+10], di
    mov [{regs}+12], bp
    mov word ptr [{regs}+16], es

rm_to_pm:
    xor ax, ax
    mov ds, ax
    mov es, ax
    mov ss, ax
    mov eax, cr0
    or eax, {pe_bit}
    mov cr0, eax
    jmp fword ptr [{far_pm16_out}]

    .code16
rm_entry_wide:
    cli
    mov es, [{wide}+16]
    mov di, [{wide}+18]
    mov si, [{wide}+22]
    mov ax, [{wide}]
    mov bx, [{wide}+4]
    mov cx, [{wide}+8]
    mov edx, [{wide}+12]
    int 0x15
wide_store:
    mov [{wide}], eax
    mov [{wide}+4], bx
    mov [{wide}+8], cx
    pushfd
    pop ax
    mov [{wide}+20], ax
    jmp rm_to_pm

pm16_return:
    mov ax, {data32}
    mov ds, ax
    mov es, ax
    mov fs, ax
    mov gs, ax
    mov ss, ax
    jmp fword ptr [{far_pm32_out}]

    .code32
thunk32_return:
    mov esi, [{ret_src}]
    mov edi, [{regs_ptr}]
    mov ecx, [{ret_cnt}]
    rep movsb

restore_frame:
    mov esp, [{saved_esp}]
    pop edi
    pop esi
    pop ebp
    pop ebx
    push dword ptr [{saved_eflags}]
    popfd
    ret

    .skip 0x120 - (. - thunk_blob_start)
    .long pm16_landing - thunk_blob_start + {tramp}
    .short {code16}
    .long rm_entry - thunk_blob_start + {tramp}
    .short 0x0000
    .long rm_entry_wide - thunk_blob_start + {tramp}
    .short 0x0000
    .long pm16_return - thunk_blob_start + {tramp}
    .short {code16}
    .long thunk32_return - thunk_blob_start + {tramp}
    .short {code32}
    .skip 0x148 - (. - thunk_blob_start)
    .skip 0x18
    .skip 0x20
    .skip 0xC
    .skip 0x200 - (. - thunk_blob_start)
    .globl thunk_blob_end
thunk_blob_end:

    .text
    .code32
    .globl thunk_entry32
thunk_entry32:
    push ebx
    push ebp
    push esi
    push edi
    mov dword ptr [{regs_ptr}], edx
    mov byte ptr [{int_no}], al
    mov dword ptr [{saved_esp}], esp
    pushfd
    pop dword ptr [{saved_eflags}]
    mov eax, [edx]
    mov [{regs}], eax
    mov eax, [edx+4]
    mov [{regs}+4], eax
    mov eax, [edx+8]
    mov [{regs}+8], eax
    mov eax, [edx+12]
    mov [{regs}+12], eax
    mov ax, [edx+16]
    mov [{regs}+16], ax
    mov dword ptr [{ret_src}], {regs}
    mov dword ptr [{ret_cnt}], 18
    jmp fword ptr [{far_pm16_in}]

    .code32
    .globl thunk_entry32_wide
thunk_entry32_wide:
    push ebx
    push ebp
    push esi
    push edi
    mov byte ptr [{int_no}], {wide_mark}
    mov dword ptr [{regs_ptr}], edx
    mov dword ptr [{saved_esp}], esp
    pushfd
    pop dword ptr [{saved_eflags}]
    mov eax, [edx]
    mov [{wide}], eax
    mov eax, [edx+4]
    mov [{wide}+4], eax
    mov eax, [edx+8]
    mov [{wide}+8], eax
    mov eax, [edx+12]
    mov [{wide}+12], eax
    mov ax, [edx+16]
    mov [{wide}+16], ax
    mov ax, [edx+18]
    mov [{wide}+18], ax
    mov ax, [edx+22]
    mov [{wide}+22], ax
    mov dword ptr [{ret_src}], {wide}
    mov dword ptr [{ret_cnt}], 24
    jmp fword ptr [{far_pm16_in}]
    "#,
    tramp = const TRAMPOLINE_BASE as u32,
    far_pm16_in = const M_FAR_PM16_IN,
    far_rm = const M_FAR_RM,
    far_rm_wide = const M_FAR_RM_WIDE,
    far_pm16_out = const M_FAR_PM16_OUT,
    far_pm32_out = const M_FAR_PM32_OUT,
    regs = const M_REGS,
    wide = const M_WIDE,
    saved_esp = const M_SAVED_ESP,
    regs_ptr = const M_REGS_PTR,
    int_no = const M_INT_NO,
    saved_eflags = const M_SAVED_EFLAGS,
    ret_src = const M_RET_SRC,
    ret_cnt = const M_RET_CNT,
    rm_stack = const RM_STACK_TOP,
    pe_bit = const PE_BIT,
    pe_clear_mask = const PE_CLEAR_MASK,
    wide_mark = const WIDE_MARK as u8,
    code32 = const 0x08u32,
    code16 = const 0x18u32,
    data32 = const 0x10u32,
);
