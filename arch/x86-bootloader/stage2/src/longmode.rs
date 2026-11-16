use rosx_boot_common::DEBUG_EXIT_FAILURE;
use rosx_boot_common::debug_exit;
use rosx_boot_common::serial::Serial;

const EXTENDED_LEAF_FLOOR: u32 = 0x8000_0000;
const EXTENDED_FEATURES_LEAF: u32 = 0x8000_0001;
const LONG_MODE_BIT: u32 = 1 << 29;

pub fn ensure_long_mode(console: &Serial) {
    if !cpuid_available() || !long_mode_supported() {
        console.write_line("[ERR] CPU does not support x86_64 Long Mode");
        debug_exit(DEBUG_EXIT_FAILURE);
    }
}

fn long_mode_supported() -> bool {
    cpuid(EXTENDED_LEAF_FLOOR).0 >= EXTENDED_FEATURES_LEAF
        && cpuid(EXTENDED_FEATURES_LEAF).3 & LONG_MODE_BIT != 0
}

fn cpuid_available() -> bool {
    let mut original = 0u32;
    let mut toggled = 0u32;
    unsafe {
        core::arch::asm!(
            "pushfd",
            "pop {original}",
            "mov {toggled}, {original}",
            "xor {toggled}, 0x200000",
            "push {toggled}",
            "popfd",
            "pushfd",
            "pop {toggled}",
            "push {original}",
            "popfd",
            original = inout(reg) original,
            toggled = inout(reg) toggled,
        )
    };
    original != toggled
}

#[inline(never)]
#[unsafe(link_section = ".longmode.text")]
fn cpuid(leaf: u32) -> (u32, u32, u32, u32) {
    let (eax, ebx, ecx, edx): (u32, u32, u32, u32);
    unsafe {
        core::arch::asm!(
            "cpuid",
            inout("eax") leaf => eax,
            lateout("ebx") ebx,
            lateout("ecx") ecx,
            lateout("edx") edx,
        )
    };
    (eax, ebx, ecx, edx)
}
