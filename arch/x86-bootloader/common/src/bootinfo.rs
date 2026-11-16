use system::boot_info::BOOT_INFO_MAGIC;
use system::boot_info::BOOT_INFO_VERSION;
use system::boot_info::BootArch;
use system::boot_info::BootInfo;
use system::boot_info::BootMethod;
use system::boot_info::DisplayInfo;
use system::boot_info::DisplayKind;
use system::boot_info::HIGHER_HALF_OFFSET;
use system::boot_info::MemoryRegion;

pub const BOOT_INFO_WINDOW_ADDRESS: u64 = 0x0003_0000;
pub const BOOT_INFO_WINDOW_BYTES: u64 = 0x1000;

pub const BOOT_INFO_ADDRESS: u64 = (BOOT_INFO_WINDOW_ADDRESS
    + BOOT_INFO_WINDOW_BYTES
    - core::mem::size_of::<BootInfo>() as u64)
    & !15;

pub fn assemble(memory_map_len: u32, initrd_address: u64, initrd_size: u64) -> BootInfo {
    BootInfo {
        magic: BOOT_INFO_MAGIC,
        version: BOOT_INFO_VERSION,
        arch: BootArch::X86_32,
        boot_method: BootMethod::Bios,
        memory_map_ptr: BOOT_INFO_WINDOW_ADDRESS,
        memory_map_len,
        memory_map_entry_size: core::mem::size_of::<MemoryRegion>() as u32,
        initrd_addr: initrd_address,
        initrd_size,
        display: DisplayInfo {
            kind: DisplayKind::VgaText,
            addr: 0x000B_8000,
            width: 80,
            height: 25,
            pitch: 160,
            bpp: 0,
            red_mask_shift: 0,
            green_mask_shift: 0,
            blue_mask_shift: 0,
        },
    }
}

pub fn assemble_long_mode(
    memory_map_len: u32,
    initrd_address: u64,
    initrd_size: u64,
) -> BootInfo {
    let mut info = assemble(memory_map_len, initrd_address, initrd_size);
    info.arch = BootArch::X86_64;
    info.memory_map_ptr += HIGHER_HALF_OFFSET;
    if info.initrd_addr != 0 {
        info.initrd_addr += HIGHER_HALF_OFFSET;
    }
    info
}

#[cfg(test)]
mod tests {
    use super::*;
    use system::boot_info::MEMORY_MAP_MAX_ENTRIES;

    #[test]
    fn boot_info_lands_at_the_top_of_its_window() {
        assert!(BOOT_INFO_ADDRESS >= BOOT_INFO_WINDOW_ADDRESS);
        assert!(BOOT_INFO_ADDRESS + core::mem::size_of::<BootInfo>() as u64
            <= BOOT_INFO_WINDOW_ADDRESS + BOOT_INFO_WINDOW_BYTES);
    }

    #[test]
    fn boot_info_address_is_sixteen_byte_aligned() {
        assert_eq!(BOOT_INFO_ADDRESS % 16, 0);
    }

    #[test]
    fn boot_info_never_collides_with_the_full_map_array() {
        let map_end = BOOT_INFO_WINDOW_ADDRESS
            + (MEMORY_MAP_MAX_ENTRIES * core::mem::size_of::<MemoryRegion>()) as u64;
        assert!(BOOT_INFO_ADDRESS >= map_end);
    }

    #[test]
    fn assemble_fills_the_treaty_identity() {
        let info = assemble(11, 0x0010_1000, 65536);
        assert_eq!(info.magic, BOOT_INFO_MAGIC);
        assert_eq!(info.version, BOOT_INFO_VERSION);
        assert_eq!(info.arch, BootArch::X86_32);
        assert_eq!(info.boot_method, BootMethod::Bios);
    }

    #[test]
    fn assemble_carries_the_map_at_its_physical_home_with_the_real_stride() {
        let info = assemble(11, 0x0010_1000, 65536);
        assert_eq!(info.memory_map_ptr, BOOT_INFO_WINDOW_ADDRESS);
        assert_eq!(info.memory_map_len, 11);
        assert_eq!(
            info.memory_map_entry_size,
            core::mem::size_of::<MemoryRegion>() as u32
        );
    }

    #[test]
    fn assemble_passes_the_initrd_fields_through() {
        let info = assemble(9, 0x0010_1000, 65536);
        assert_eq!(info.initrd_addr, 0x0010_1000);
        assert_eq!(info.initrd_size, 65536);
        let empty = assemble(9, 0, 0);
        assert_eq!(empty.initrd_addr, 0);
        assert_eq!(empty.initrd_size, 0);
    }

    #[test]
    fn assemble_declares_vga_text_per_the_abi_table() {
        let info = assemble(1, 0, 0);
        assert_eq!(info.display.kind, DisplayKind::VgaText);
        assert_eq!(info.display.addr, 0x000B_8000);
        assert_eq!(info.display.width, 80);
        assert_eq!(info.display.height, 25);
        assert_eq!(info.display.pitch, 160);
        assert_eq!(info.display.bpp, 0);
    }

    #[test]
    fn long_mode_assembly_declares_x86_64_and_the_direct_map() {
        let info = assemble_long_mode(12, 0x0010_1000, 65536);
        assert_eq!(info.arch, BootArch::X86_64);
        assert_eq!(
            info.memory_map_ptr,
            BOOT_INFO_WINDOW_ADDRESS + HIGHER_HALF_OFFSET
        );
        assert_eq!(info.initrd_addr, 0x0010_1000 + HIGHER_HALF_OFFSET);
        assert_eq!(info.initrd_size, 65536);
        assert_eq!(info.magic, BOOT_INFO_MAGIC);
    }

    #[test]
    fn an_absent_initrd_keeps_the_zero_address_in_long_mode_too() {
        let empty = assemble_long_mode(9, 0, 0);
        assert_eq!(empty.initrd_addr, 0);
        assert_eq!(empty.initrd_size, 0);
    }
}
