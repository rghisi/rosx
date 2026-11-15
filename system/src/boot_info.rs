pub const BOOT_INFO_MAGIC: u32 = 0x524F_5358;
pub const BOOT_INFO_VERSION: u16 = 1;
pub const HIGHER_HALF_OFFSET: u64 = 0xFFFF_8000_0000_0000;
pub const MEMORY_MAP_MAX_ENTRIES: usize = 64;

#[repr(u16)]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum BootArch {
    X86_32 = 1,
    X86_64 = 2,
}

#[repr(u16)]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum BootMethod {
    Bios = 1,
    Uefi = 2,
}

#[repr(u32)]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum MemoryKind {
    Usable = 1,
    Reserved = 2,
    BootloaderReclaimable = 3,
    KernelAndModules = 5,
}

#[repr(C)]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct MemoryRegion {
    pub base: u64,
    pub length: u64,
    pub kind: MemoryKind,
}

#[repr(u32)]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum DisplayKind {
    None = 0,
    VgaText = 1,
    LinearFramebuffer = 2,
}

#[repr(C)]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct DisplayInfo {
    pub kind: DisplayKind,
    pub addr: u64,
    pub width: u32,
    pub height: u32,
    pub pitch: u32,
    pub bpp: u8,
    pub red_mask_shift: u8,
    pub green_mask_shift: u8,
    pub blue_mask_shift: u8,
}

#[repr(C)]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct BootInfo {
    pub magic: u32,
    pub version: u16,
    pub arch: BootArch,
    pub boot_method: BootMethod,
    pub memory_map_ptr: u64,
    pub memory_map_len: u32,
    pub memory_map_entry_size: u32,
    pub initrd_addr: u64,
    pub initrd_size: u64,
    pub display: DisplayInfo,
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::mem::{align_of, offset_of, size_of};

    #[test]
    fn magic_is_rose_in_ascii() {
        assert_eq!(BOOT_INFO_MAGIC, u32::from_be_bytes(*b"ROSX"));
    }

    #[test]
    fn higher_half_offset_is_the_canonical_sign_extension() {
        assert_eq!(HIGHER_HALF_OFFSET, -0x0000_8000_0000_0000i64 as u64);
        assert_eq!(HIGHER_HALF_OFFSET >> 48, 0xFFFF);
    }

    #[test]
    fn memory_map_capacity_is_sixty_four() {
        assert_eq!(MEMORY_MAP_MAX_ENTRIES, 64);
    }

    #[test]
    fn enum_discriminants_match_the_frozen_wire_values() {
        assert_eq!(BootArch::X86_32 as u16, 1);
        assert_eq!(BootArch::X86_64 as u16, 2);
        assert_eq!(BootMethod::Bios as u16, 1);
        assert_eq!(BootMethod::Uefi as u16, 2);
        assert_eq!(MemoryKind::Usable as u32, 1);
        assert_eq!(MemoryKind::Reserved as u32, 2);
        assert_eq!(MemoryKind::BootloaderReclaimable as u32, 3);
        assert_eq!(MemoryKind::KernelAndModules as u32, 5);
        assert_eq!(DisplayKind::None as u32, 0);
        assert_eq!(DisplayKind::VgaText as u32, 1);
        assert_eq!(DisplayKind::LinearFramebuffer as u32, 2);
    }

    #[test]
    fn memory_kind_four_stays_retired() {
        let retired = 4u32;
        assert_ne!(retired, MemoryKind::KernelAndModules as u32);
        let variants = [
            MemoryKind::Usable as u32,
            MemoryKind::Reserved as u32,
            MemoryKind::BootloaderReclaimable as u32,
            MemoryKind::KernelAndModules as u32,
        ];
        assert!(!variants.contains(&retired));
    }

    #[test]
    fn memory_region_is_a_packed_triple() {
        assert_eq!(size_of::<MemoryRegion>(), 24);
        assert_eq!(align_of::<MemoryRegion>(), 8);
        assert_eq!(offset_of!(MemoryRegion, base), 0);
        assert_eq!(offset_of!(MemoryRegion, length), 8);
        assert_eq!(offset_of!(MemoryRegion, kind), 16);
    }

    #[test]
    fn display_info_layout_is_pinned() {
        assert_eq!(size_of::<DisplayInfo>(), 32);
        assert_eq!(align_of::<DisplayInfo>(), 8);
        assert_eq!(offset_of!(DisplayInfo, kind), 0);
        assert_eq!(offset_of!(DisplayInfo, addr), 8);
        assert_eq!(offset_of!(DisplayInfo, width), 16);
        assert_eq!(offset_of!(DisplayInfo, height), 20);
        assert_eq!(offset_of!(DisplayInfo, pitch), 24);
        assert_eq!(offset_of!(DisplayInfo, bpp), 28);
    }

    #[test]
    fn boot_info_layout_is_pinned() {
        assert_eq!(size_of::<BootInfo>(), 80);
        assert_eq!(align_of::<BootInfo>(), 8);
        assert_eq!(offset_of!(BootInfo, magic), 0);
        assert_eq!(offset_of!(BootInfo, version), 4);
        assert_eq!(offset_of!(BootInfo, arch), 6);
        assert_eq!(offset_of!(BootInfo, boot_method), 8);
        assert_eq!(offset_of!(BootInfo, memory_map_ptr), 16);
        assert_eq!(offset_of!(BootInfo, memory_map_len), 24);
        assert_eq!(offset_of!(BootInfo, memory_map_entry_size), 28);
        assert_eq!(offset_of!(BootInfo, initrd_addr), 32);
        assert_eq!(offset_of!(BootInfo, initrd_size), 40);
        assert_eq!(offset_of!(BootInfo, display), 48);
    }

    #[test]
    fn boot_info_is_movable_by_copy() {
        fn assert_copy<T: Copy>() {}
        assert_copy::<BootInfo>();
        assert_copy::<MemoryRegion>();
        assert_copy::<DisplayInfo>();
    }
}
