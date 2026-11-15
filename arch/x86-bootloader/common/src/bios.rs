pub const TRAMPOLINE_BASE: usize = 0x600;
pub const TRAMPOLINE_BUDGET: usize = 0x200;
pub const TRAMPOLINE_STABLE_BYTES: usize = 0x144;
pub const BOOT_DRIVE_MEMORY: usize = 0x5FF;

#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
#[repr(C)]
pub struct Regs16 {
    pub ax: u16,
    pub bx: u16,
    pub cx: u16,
    pub dx: u16,
    pub si: u16,
    pub di: u16,
    pub bp: u16,
    pub flags: u16,
}

impl Regs16 {
    pub const fn new(ax: u16, bx: u16, cx: u16, dx: u16) -> Self {
        Self {
            ax,
            bx,
            cx,
            dx,
            si: 0,
            di: 0,
            bp: 0,
            flags: 0,
        }
    }

    pub const fn carry(&self) -> bool {
        self.flags & 1 != 0
    }
}

#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
#[repr(C)]
pub struct Regs32 {
    pub eax: u32,
    pub ebx: u32,
    pub ecx: u32,
    pub edx: u32,
    pub es: u16,
    pub di: u16,
    pub flags: u16,
    pub reserved: u16,
}

impl Regs32 {
    pub const fn new(eax: u32, ebx: u32, ecx: u32, edx: u32) -> Self {
        Self {
            eax,
            ebx,
            ecx,
            edx,
            es: 0,
            di: 0,
            flags: 0,
            reserved: 0,
        }
    }

    pub const fn carry(&self) -> bool {
        self.flags & 1 != 0
    }
}

pub fn checksum(bytes: &[u8]) -> u16 {
    let mut sum: u16 = 0;
    for byte in bytes {
        sum = sum.wrapping_add(*byte as u16);
    }
    sum
}

/// # Safety
/// The trampoline window must have been installed before reading it.
pub unsafe fn region_checksum() -> u16 {
    let window =
        unsafe { core::slice::from_raw_parts(TRAMPOLINE_BASE as *const u8, TRAMPOLINE_BUDGET) };
    checksum(window)
}

/// # Safety
/// The trampoline window must have been installed before reading it.
pub unsafe fn stable_region_checksum() -> u16 {
    let window = unsafe {
        core::slice::from_raw_parts(TRAMPOLINE_BASE as *const u8, TRAMPOLINE_STABLE_BYTES)
    };
    checksum(window)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checksum_of_empty_window_is_zero() {
        assert_eq!(checksum(&[]), 0);
    }

    #[test]
    fn checksum_sums_bytes_in_order() {
        assert_eq!(checksum(&[1, 2, 3]), 6);
        assert_eq!(checksum(&[0xFF, 0x01]), 0x100);
    }

    #[test]
    fn checksum_wraps_at_u16() {
        assert_eq!(checksum(&[0xFF; 512]), (512u32 * 255) as u16);
    }

    #[test]
    fn carry_reads_lowest_flag_bit() {
        assert!(!Regs16 { flags: 0, ..Default::default() }.carry());
        assert!(Regs16 { flags: 1, ..Default::default() }.carry());
        assert!(Regs16 { flags: 0x2001, ..Default::default() }.carry());
    }

    #[test]
    fn trampoline_window_is_a_page_pair() {
        assert_eq!(TRAMPOLINE_BASE % 0x100, 0);
        assert_eq!(TRAMPOLINE_BUDGET, 0x200);
        assert_eq!(TRAMPOLINE_BASE + TRAMPOLINE_BUDGET, 0x800);
        assert!(BOOT_DRIVE_MEMORY < TRAMPOLINE_BASE);
    }

    #[test]
    fn stable_region_covers_code_and_far_table() {
        assert_eq!(TRAMPOLINE_STABLE_BYTES, 0x144);
        assert!(TRAMPOLINE_STABLE_BYTES < TRAMPOLINE_BUDGET);
    }

    #[test]
    fn regs32_is_a_24_byte_marshalling_block() {
        assert_eq!(core::mem::size_of::<Regs32>(), 24);
        let mut regs = Regs32::new(0xE820, 0, 20, 0x534D_4150);
        regs.es = 0x4000;
        regs.di = 0x18;
        assert_eq!(regs.eax, 0xE820);
        assert_eq!(regs.edx, 0x534D_4150);
        assert!(!regs.carry());
    }
}
