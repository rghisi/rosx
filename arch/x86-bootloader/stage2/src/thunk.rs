use rosx_boot_common::bios::Regs16;
use rosx_boot_common::bios::TRAMPOLINE_BASE;
use rosx_boot_common::bios::TRAMPOLINE_BUDGET;
use rosx_boot_common::bios::checksum;
use rosx_boot_common::bios::region_checksum;
use rosx_boot_common::bios::stable_region_checksum;

const M_FAR_PM16_IN: u32 = TRAMPOLINE_BASE as u32 + 0x100;
const M_FAR_RM: u32 = TRAMPOLINE_BASE as u32 + 0x106;
const M_FAR_PM16_OUT: u32 = TRAMPOLINE_BASE as u32 + 0x10C;
const M_FAR_PM32_OUT: u32 = TRAMPOLINE_BASE as u32 + 0x112;
const M_REGS: u32 = TRAMPOLINE_BASE as u32 + 0x120;
const M_SAVED_ESP: u32 = TRAMPOLINE_BASE as u32 + 0x130;
const M_REGS_PTR: u32 = TRAMPOLINE_BASE as u32 + 0x134;
const M_INT_NO: u32 = TRAMPOLINE_BASE as u32 + 0x138;
const M_SAVED_EFLAGS: u32 = TRAMPOLINE_BASE as u32 + 0x13C;
const RM_STACK_TOP: u32 = 0x7A00;
const PE_BIT: u32 = 1;
const PE_CLEAR_MASK: u32 = 0x7FFF_FFFE;

unsafe extern "C" {
    fn thunk_entry32();
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
    mov esp, {rm_stack}
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
    jmp fword ptr [{far_rm}]

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
    cmp byte ptr [{int_no}], 0x13
    je thunk_int13
    cmp byte ptr [{int_no}], 0x15
    je thunk_int15
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
    xor ax, ax
    mov ds, ax
    mov es, ax
    mov ss, ax
    mov eax, cr0
    or eax, {pe_bit}
    mov cr0, eax
    jmp fword ptr [{far_pm16_out}]

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
    mov eax, [{regs_ptr}]
    mov ecx, [{regs}]
    mov [eax], ecx
    mov ecx, [{regs}+4]
    mov [eax+4], ecx
    mov ecx, [{regs}+8]
    mov [eax+8], ecx
    mov ecx, [{regs}+12]
    mov [eax+12], ecx
    mov esp, [{saved_esp}]
    pop edi
    pop esi
    pop ebp
    pop ebx
    push dword ptr [{saved_eflags}]
    popfd
    ret

    .skip 0x100 - (. - thunk_blob_start)
    .long pm16_landing - thunk_blob_start + {tramp}
    .short {code16}
    .long rm_entry - thunk_blob_start + {tramp}
    .short 0x0000
    .long pm16_return - thunk_blob_start + {tramp}
    .short {code16}
    .long thunk32_return - thunk_blob_start + {tramp}
    .short {code32}
    .skip 0x120 - (. - thunk_blob_start)
    .skip 0x20
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
    jmp fword ptr [{far_pm16_in}]
    "#,
    tramp = const TRAMPOLINE_BASE as u32,
    far_pm16_in = const M_FAR_PM16_IN,
    far_rm = const M_FAR_RM,
    far_pm16_out = const M_FAR_PM16_OUT,
    far_pm32_out = const M_FAR_PM32_OUT,
    regs = const M_REGS,
    saved_esp = const M_SAVED_ESP,
    regs_ptr = const M_REGS_PTR,
    int_no = const M_INT_NO,
    saved_eflags = const M_SAVED_EFLAGS,
    rm_stack = const RM_STACK_TOP,
    pe_bit = const PE_BIT,
    pe_clear_mask = const PE_CLEAR_MASK,
    code32 = const 0x08u32,
    code16 = const 0x18u32,
    data32 = const 0x10u32,
);
