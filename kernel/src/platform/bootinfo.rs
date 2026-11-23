use core::mem::offset_of;
use core::mem::size_of;

use crate::memory::MAX_MEMORY_BLOCKS;
use crate::memory::MemoryBlock;
use crate::memory::MemoryBlocks;
use system::boot_info::BOOT_INFO_MAGIC;
use system::boot_info::BOOT_INFO_VERSION;
use system::boot_info::BootArch;
use system::boot_info::BootInfo;
use system::boot_info::DisplayInfo;
use system::boot_info::DisplayKind;
use system::boot_info::HIGHER_HALF_OFFSET;
use system::boot_info::MEMORY_MAP_MAX_ENTRIES;
use system::boot_info::MemoryKind;
use system::boot_info::MemoryRegion;

const _: () = assert!(size_of::<MemoryRegion>() == 24);
const _: () = assert!(size_of::<BootInfo>() >= offset_of!(BootInfo, memory_map_entry_size) + 4);

const VGA_TEXT_ADDRESS: u64 = 0x000B_8000;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum BootInfoError {
    Magic,
    Version,
    Arch,
    EntrySize,
    MapLen,
    MapPtr,
    Display,
    Initrd,
    InitrdRange,
    InitrdNotReserved,
}

#[derive(Copy, Clone, Debug)]
pub struct MemoryMapFacts {
    pub regions: u32,
    pub usable_bytes: u64,
    pub accepted: usize,
    pub clipped: usize,
    pub dropped: usize,
}

/// Converts the frozen BootInfo treaty at a physical address into kernel memory blocks,
/// validating every wire field before dereferencing the map and decoding no enum except
/// through `MemoryKind::from_wire`; the caller guarantees the header and map bytes stay
/// readable. See docs/rationale/x86-bootinfo-consumer.md.
pub unsafe fn usable_blocks(
    address: usize,
    expect_arch: BootArch,
    kernel_end: usize,
    blocks: &mut MemoryBlocks,
) -> Result<MemoryMapFacts, BootInfoError> {
    let header =
        unsafe { core::slice::from_raw_parts(address as *const u8, size_of::<BootInfo>()) };
    if u32_at(header, offset_of!(BootInfo, magic)) != BOOT_INFO_MAGIC {
        return Err(BootInfoError::Magic);
    }
    if u16_at(header, offset_of!(BootInfo, version)) != BOOT_INFO_VERSION {
        return Err(BootInfoError::Version);
    }
    if u16_at(header, offset_of!(BootInfo, arch)) != expect_arch as u16 {
        return Err(BootInfoError::Arch);
    }
    let entry_size = u32_at(header, offset_of!(BootInfo, memory_map_entry_size));
    if entry_size != size_of::<MemoryRegion>() as u32 {
        return Err(BootInfoError::EntrySize);
    }
    let regions = u32_at(header, offset_of!(BootInfo, memory_map_len));
    if regions > MEMORY_MAP_MAX_ENTRIES as u32 {
        return Err(BootInfoError::MapLen);
    }
    let map_address = u64_at(header, offset_of!(BootInfo, memory_map_ptr));
    if map_address == 0 || usize::try_from(map_address).is_err() {
        return Err(BootInfoError::MapPtr);
    }
    let display = offset_of!(BootInfo, display);
    let display_kind = u32_at(header, display + offset_of!(DisplayInfo, kind));
    let display_addr = u64_at(header, display + offset_of!(DisplayInfo, addr));
    if display_kind != DisplayKind::VgaText as u32 || display_addr != VGA_TEXT_ADDRESS {
        return Err(BootInfoError::Display);
    }
    let map = unsafe {
        core::slice::from_raw_parts(
            map_address as *const u8,
            regions as usize * entry_size as usize,
        )
    };
    let mut facts = MemoryMapFacts {
        regions,
        usable_bytes: 0,
        accepted: 0,
        clipped: 0,
        dropped: 0,
    };
    for index in 0..regions as usize {
        let record = &map[index * entry_size as usize..(index + 1) * entry_size as usize];
        let kind = u32_at(record, offset_of!(MemoryRegion, kind));
        if MemoryKind::from_wire(kind) != Some(MemoryKind::Usable) {
            continue;
        }
        let base = u64_at(record, offset_of!(MemoryRegion, base));
        let length = u64_at(record, offset_of!(MemoryRegion, length));
        facts.usable_bytes = facts.usable_bytes.saturating_add(length);
        let Some(end) = base.checked_add(length) else {
            facts.dropped += 1;
            continue;
        };
        if length == 0 || end > usize::MAX as u64 {
            facts.dropped += 1;
            continue;
        }
        let floor = kernel_end as u64;
        if end <= floor {
            facts.clipped += 1;
            continue;
        }
        let start = base.max(floor);
        if start != base {
            facts.clipped += 1;
        }
        if blocks.count == MAX_MEMORY_BLOCKS {
            facts.dropped += 1;
            continue;
        }
        blocks.blocks[blocks.count] = MemoryBlock::free(
            start as usize,
            (end - start) as usize,
        );
        blocks.count += 1;
        facts.accepted += 1;
    }
    Ok(facts)
}

/// Reads the initrd range out of the frozen BootInfo treaty and returns it as a slice:
/// the ABI row (zero size pairs with zero address) and the address-space width are
/// enforced, and the whole physical range must lie inside one `KernelAndModules` region
/// of the published map, so the allocator can never hand out initrd bytes. The caller
/// guarantees the header, map and initrd bytes stay readable.
/// See docs/rationale/x86-bootinfo-consumer.md.
pub unsafe fn initrd(
    address: usize,
    expect_arch: BootArch,
) -> Result<Option<&'static [u8]>, BootInfoError> {
    let header =
        unsafe { core::slice::from_raw_parts(address as *const u8, size_of::<BootInfo>()) };
    if u32_at(header, offset_of!(BootInfo, magic)) != BOOT_INFO_MAGIC {
        return Err(BootInfoError::Magic);
    }
    if u16_at(header, offset_of!(BootInfo, version)) != BOOT_INFO_VERSION {
        return Err(BootInfoError::Version);
    }
    if u16_at(header, offset_of!(BootInfo, arch)) != expect_arch as u16 {
        return Err(BootInfoError::Arch);
    }
    let wire_address = u64_at(header, offset_of!(BootInfo, initrd_addr));
    let size = u64_at(header, offset_of!(BootInfo, initrd_size));
    if size == 0 || wire_address == 0 {
        return if size == 0 && wire_address == 0 {
            Ok(None)
        } else {
            Err(BootInfoError::Initrd)
        };
    }
    let entry_size = u32_at(header, offset_of!(BootInfo, memory_map_entry_size));
    if entry_size != size_of::<MemoryRegion>() as u32 {
        return Err(BootInfoError::EntrySize);
    }
    let regions = u32_at(header, offset_of!(BootInfo, memory_map_len));
    if regions > MEMORY_MAP_MAX_ENTRIES as u32 {
        return Err(BootInfoError::MapLen);
    }
    let map_address = u64_at(header, offset_of!(BootInfo, memory_map_ptr));
    if map_address == 0 || usize::try_from(map_address).is_err() {
        return Err(BootInfoError::MapPtr);
    }
    let physical = match expect_arch {
        BootArch::X86_64 => wire_address.checked_sub(HIGHER_HALF_OFFSET),
        BootArch::X86_32 => Some(wire_address),
    };
    let Some(physical) = physical else {
        return Err(BootInfoError::InitrdRange);
    };
    let Some(physical_end) = physical.checked_add(size) else {
        return Err(BootInfoError::InitrdRange);
    };
    let width_limit = usize::MAX as u64;
    if wire_address > width_limit || physical > width_limit || physical_end > width_limit {
        return Err(BootInfoError::InitrdRange);
    }
    let slice = unsafe {
        let map = core::slice::from_raw_parts(
            map_address as *const u8,
            regions as usize * entry_size as usize,
        );
        let mut reserved = false;
        for index in 0..regions as usize {
            let record = &map[index * entry_size as usize..(index + 1) * entry_size as usize];
            let kind = u32_at(record, offset_of!(MemoryRegion, kind));
            if MemoryKind::from_wire(kind) != Some(MemoryKind::KernelAndModules) {
                continue;
            }
            let base = u64_at(record, offset_of!(MemoryRegion, base));
            let length = u64_at(record, offset_of!(MemoryRegion, length));
            let Some(region_end) = base.checked_add(length) else {
                continue;
            };
            if base <= physical && physical_end <= region_end {
                reserved = true;
                break;
            }
        }
        if !reserved {
            return Err(BootInfoError::InitrdNotReserved);
        }
        core::slice::from_raw_parts(wire_address as usize as *const u8, size as usize)
    };
    Ok(Some(slice))
}

fn u16_at(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes(bytes[offset..offset + 2].try_into().unwrap())
}

fn u32_at(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
}

fn u64_at(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap())
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAP_OFFSET: usize = 0x100;
    const INFO_OFFSET: usize = 0x800;
    const KERNEL_END: usize = 0x001c_0000;

    fn window() -> Vec<u8> {
        vec![0u8; INFO_OFFSET + size_of::<BootInfo>()]
    }

    fn header(
        buf: &mut [u8],
        map_len: u32,
        entry_size: u32,
        map_ptr: u64,
        arch: BootArch,
        version: u16,
        magic: u32,
    ) {
        let info = &mut buf[INFO_OFFSET..];
        info[offset_of!(BootInfo, magic)..][..4].copy_from_slice(&magic.to_le_bytes());
        info[offset_of!(BootInfo, version)..][..2].copy_from_slice(&version.to_le_bytes());
        info[offset_of!(BootInfo, arch)..][..2].copy_from_slice(&(arch as u16).to_le_bytes());
        info[offset_of!(BootInfo, memory_map_ptr)..][..8].copy_from_slice(&map_ptr.to_le_bytes());
        info[offset_of!(BootInfo, memory_map_len)..][..4]
            .copy_from_slice(&map_len.to_le_bytes());
        info[offset_of!(BootInfo, memory_map_entry_size)..][..4]
            .copy_from_slice(&entry_size.to_le_bytes());
        let display = offset_of!(BootInfo, display);
        info[display + offset_of!(DisplayInfo, kind)..][..4]
            .copy_from_slice(&(DisplayKind::VgaText as u32).to_le_bytes());
        info[display + offset_of!(DisplayInfo, addr)..][..8]
            .copy_from_slice(&VGA_TEXT_ADDRESS.to_le_bytes());
    }

    fn stage2_header(buf: &mut [u8], map_len: u32) {
        let map_address = (MAP_OFFSET + buf.as_ptr() as usize) as u64;        header(
            buf,
            map_len,
            size_of::<MemoryRegion>() as u32,
            map_address,
            BootArch::X86_32,
            BOOT_INFO_VERSION,
            BOOT_INFO_MAGIC,
        );
    }

    fn region(buf: &mut [u8], index: usize, base: u64, length: u64, kind: u32) {
        let slot = MAP_OFFSET + index * size_of::<MemoryRegion>();
        buf[slot..][..8].copy_from_slice(&base.to_le_bytes());
        buf[slot + 8..][..8].copy_from_slice(&length.to_le_bytes());
        buf[slot + 16..][..4].copy_from_slice(&kind.to_le_bytes());
    }

    fn empty_blocks() -> MemoryBlocks {
        MemoryBlocks {
            blocks: core::array::from_fn(|_| MemoryBlock::free(0, 0)),
            count: 0,
        }
    }

    fn consume(buf: &[u8]) -> (Result<MemoryMapFacts, BootInfoError>, MemoryBlocks) {
        let mut blocks = empty_blocks();
        let outcome = unsafe {
            usable_blocks(
                buf.as_ptr() as usize + INFO_OFFSET,
                BootArch::X86_32,
                KERNEL_END,
                &mut blocks,
            )
        };
        (outcome, blocks)
    }

    #[test]
    fn refuses_a_window_without_the_rosx_magic() {
        let buf = window();
        let (outcome, blocks) = consume(&buf);
        assert_eq!(outcome.unwrap_err(), BootInfoError::Magic);
        assert_eq!(blocks.count, 0);
    }

    #[test]
    fn refuses_a_version_or_arch_the_treaty_does_not_offer() {
        let mut buf = window();
        stage2_header(&mut buf, 0);
        buf[INFO_OFFSET + offset_of!(BootInfo, version)..][..2]
            .copy_from_slice(&(BOOT_INFO_VERSION + 1).to_le_bytes());
        assert_eq!(consume(&buf).0.unwrap_err(), BootInfoError::Version);

        let mut buf = window();
        let base = buf.as_ptr() as usize;
        header(
            &mut buf,
            0,
            size_of::<MemoryRegion>() as u32,
            (MAP_OFFSET + base) as u64,
            BootArch::X86_64,
            BOOT_INFO_VERSION,
            BOOT_INFO_MAGIC,
        );
        assert_eq!(consume(&buf).0.unwrap_err(), BootInfoError::Arch);
    }

    #[test]
    fn refuses_a_map_the_treaty_cannot_describe() {
        let mut buf = window();
        stage2_header(&mut buf, 0);
        buf[INFO_OFFSET + offset_of!(BootInfo, memory_map_entry_size)..][..4]
            .copy_from_slice(&20u32.to_le_bytes());
        assert_eq!(consume(&buf).0.unwrap_err(), BootInfoError::EntrySize);

        let mut buf = window();
        stage2_header(&mut buf, MEMORY_MAP_MAX_ENTRIES as u32 + 1);
        assert_eq!(consume(&buf).0.unwrap_err(), BootInfoError::MapLen);

        let mut info = [0u8; size_of::<BootInfo>()];
        info[offset_of!(BootInfo, magic)..][..4].copy_from_slice(&BOOT_INFO_MAGIC.to_le_bytes());
        info[offset_of!(BootInfo, version)..][..2].copy_from_slice(&BOOT_INFO_VERSION.to_le_bytes());
        info[offset_of!(BootInfo, arch)..][..2].copy_from_slice(&(BootArch::X86_32 as u16).to_le_bytes());
        info[offset_of!(BootInfo, memory_map_entry_size)..][..4]
            .copy_from_slice(&(size_of::<MemoryRegion>() as u32).to_le_bytes());
        let outcome = unsafe {
            usable_blocks(
                info.as_ptr() as usize,
                BootArch::X86_32,
                KERNEL_END,
                &mut empty_blocks(),
            )
        };
        assert_eq!(outcome.unwrap_err(), BootInfoError::MapPtr);
    }

    #[test]
    fn refuses_a_display_the_vga_text_console_cannot_drive() {
        let mut buf = window();
        stage2_header(&mut buf, 0);
        let display = INFO_OFFSET + offset_of!(BootInfo, display);
        buf[display + offset_of!(DisplayInfo, kind)..][..4].copy_from_slice(&0u32.to_le_bytes());
        assert_eq!(consume(&buf).0.unwrap_err(), BootInfoError::Display);

        let mut buf = window();
        stage2_header(&mut buf, 0);
        buf[display + offset_of!(DisplayInfo, addr)..][..8]
            .copy_from_slice(&0x000A_0000u64.to_le_bytes());
        assert_eq!(consume(&buf).0.unwrap_err(), BootInfoError::Display);
    }

    #[test]
    fn takes_only_usable_regions_and_keeps_their_exact_bounds() {
        let mut buf = window();
        region(&mut buf, 0, 0x0000_0500, 0x0002_fb00, 3);
        region(&mut buf, 1, 0x001c_0000, 0x0100_0000, 1);
        region(&mut buf, 2, 0x0010_0000, 0x000c_0000, 5);
        region(&mut buf, 3, 0x0200_0000, 0x0010_0000, 1);
        stage2_header(&mut buf, 4);
        let (facts, blocks) = consume(&buf);
        let facts = facts.expect("a well-formed map");
        assert_eq!(facts.regions, 4);
        assert_eq!(facts.usable_bytes, 0x0110_0000);
        assert_eq!(facts.accepted, 2);
        assert_eq!(facts.clipped, 0);
        assert_eq!(facts.dropped, 0);
        assert_eq!(blocks.count, 2);
        assert_eq!(blocks.blocks[0].start, 0x001c_0000);
        assert_eq!(blocks.blocks[0].size, 0x0100_0000);
        assert_eq!(blocks.blocks[1].start, 0x0200_0000);
        assert_eq!(blocks.blocks[1].size, 0x0010_0000);
    }

    #[test]
    fn clips_regions_at_the_kernel_end() {
        let mut buf = window();
        region(&mut buf, 0, 0x0000_0000, 0x0010_0000, 1);
        region(&mut buf, 1, 0x0010_0000, 0x0020_0000, 1);
        stage2_header(&mut buf, 2);
        let (facts, blocks) = consume(&buf);
        let facts = facts.expect("a well-formed map");
        assert_eq!(facts.accepted, 1);
        assert_eq!(facts.clipped, 2);
        assert_eq!(blocks.count, 1);
        assert_eq!(blocks.blocks[0].start, KERNEL_END);
        assert_eq!(blocks.blocks[0].size, 0x0014_0000);
    }

    #[test]
    fn drops_usable_regions_beyond_the_block_cap_loudly() {
        let mut buf = window();
        for index in 0..40 {
            let base = 0x0100_0000 + index as u64 * 0x0000_1000;
            region(&mut buf, index, base, 0x0000_1000, 1);
        }
        stage2_header(&mut buf, 40);
        let (facts, blocks) = consume(&buf);
        let facts = facts.expect("a well-formed map");
        assert_eq!(facts.accepted, MAX_MEMORY_BLOCKS);
        assert_eq!(facts.dropped, 40 - MAX_MEMORY_BLOCKS);
        assert_eq!(blocks.count, MAX_MEMORY_BLOCKS);
    }

    #[test]
    fn drops_unaddressable_and_empty_regions() {
        let mut buf = window();
        region(&mut buf, 0, u64::MAX - 0xFF, 0x1_0000, 1);
        region(&mut buf, 1, 0x0100_0000, 0, 1);
        region(&mut buf, 2, 0x0200_0000, 0x0010_0000, 1);
        stage2_header(&mut buf, 3);
        let (facts, blocks) = consume(&buf);
        let facts = facts.expect("a well-formed map");
        assert_eq!(facts.accepted, 1);
        assert_eq!(facts.dropped, 2);
        assert_eq!(blocks.count, 1);
        assert_eq!(blocks.blocks[0].start, 0x0200_0000);
    }

    #[test]
    fn an_empty_map_is_valid_and_yields_nothing() {
        let mut buf = window();
        stage2_header(&mut buf, 0);
        let (facts, blocks) = consume(&buf);
        let facts = facts.expect("an empty map still carries the magic");
        assert_eq!(facts.regions, 0);
        assert_eq!(facts.usable_bytes, 0);
        assert_eq!(blocks.count, 0);
    }

    const INITRD_OFFSET: usize = INFO_OFFSET + size_of::<BootInfo>() + 0x40;
    const INITRD_FILL: u8 = 0x5A;

    fn initrd_window() -> Vec<u8> {
        let mut buf = vec![0u8; INITRD_OFFSET + 0x200];
        let fill = &mut buf[INITRD_OFFSET..];
        fill.iter_mut().for_each(|byte| *byte = INITRD_FILL);
        buf
    }

    fn initrd_at(buf: &mut [u8], address: u64, size: u64) {
        let info = &mut buf[INFO_OFFSET..];
        info[offset_of!(BootInfo, initrd_addr)..][..8].copy_from_slice(&address.to_le_bytes());
        info[offset_of!(BootInfo, initrd_size)..][..8].copy_from_slice(&size.to_le_bytes());
    }

    fn kernel_and_modules(buf: &mut [u8], base: u64, length: u64) {
        region(buf, 0, base, length, MemoryKind::KernelAndModules as u32);
    }

    fn probe(buf: &[u8], expect_arch: BootArch) -> Result<Option<&'static [u8]>, BootInfoError> {
        unsafe { initrd(buf.as_ptr() as usize + INFO_OFFSET, expect_arch) }
    }

    fn stage2_long_header(buf: &mut [u8], map_len: u32) {
        let base = buf.as_ptr() as usize;
        header(
            buf,
            map_len,
            size_of::<MemoryRegion>() as u32,
            (MAP_OFFSET + base) as u64,
            BootArch::X86_64,
            BOOT_INFO_VERSION,
            BOOT_INFO_MAGIC,
        );
    }

    #[test]
    fn an_absent_initrd_is_nothing() {
        let mut buf = initrd_window();
        kernel_and_modules(&mut buf, 0x0010_0000, 0x000c_0000);
        stage2_header(&mut buf, 1);
        assert_eq!(probe(&buf, BootArch::X86_32).unwrap().map(<[u8]>::len), None);
    }

    #[test]
    fn an_initrd_zero_pair_half_is_refused() {
        let mut buf = initrd_window();
        kernel_and_modules(&mut buf, 0x0010_0000, 0x000c_0000);
        initrd_at(&mut buf, 0x0010_1000, 0);
        stage2_header(&mut buf, 1);
        assert_eq!(probe(&buf, BootArch::X86_32).unwrap_err(), BootInfoError::Initrd);

        let mut buf = initrd_window();
        kernel_and_modules(&mut buf, 0x0010_0000, 0x000c_0000);
        initrd_at(&mut buf, 0, 4096);
        stage2_header(&mut buf, 1);
        assert_eq!(probe(&buf, BootArch::X86_32).unwrap_err(), BootInfoError::Initrd);
    }

    #[test]
    fn an_initrd_window_without_the_rosx_magic_is_refused() {
        let buf = initrd_window();
        assert_eq!(probe(&buf, BootArch::X86_32).unwrap_err(), BootInfoError::Magic);

        let mut buf = initrd_window();
        kernel_and_modules(&mut buf, 0x0010_0000, 0x000c_0000);
        initrd_at(&mut buf, 0x0010_1000, 0x100);
        let base = buf.as_ptr() as usize;
        header(
            &mut buf,
            1,
            size_of::<MemoryRegion>() as u32,
            (MAP_OFFSET + base) as u64,
            BootArch::X86_64,
            BOOT_INFO_VERSION,
            BOOT_INFO_MAGIC,
        );
        assert_eq!(probe(&buf, BootArch::X86_32).unwrap_err(), BootInfoError::Arch);
    }

    #[test]
    fn an_initrd_range_that_leaves_the_address_space_is_refused() {
        let mut buf = initrd_window();
        kernel_and_modules(&mut buf, 0x0010_0000, 0x000c_0000);
        initrd_at(&mut buf, u64::MAX - 0xFF, 0x1_0000);
        stage2_header(&mut buf, 1);
        assert_eq!(
            probe(&buf, BootArch::X86_32).unwrap_err(),
            BootInfoError::InitrdRange
        );

        let mut buf = initrd_window();
        kernel_and_modules(&mut buf, 0x0010_0000, 0x000c_0000);
        initrd_at(&mut buf, 0x0010_1000, 0x1000);
        stage2_long_header(&mut buf, 1);
        assert_eq!(
            probe(&buf, BootArch::X86_64).unwrap_err(),
            BootInfoError::InitrdRange
        );
    }

    #[test]
    fn an_initrd_straddling_a_usable_region_is_refused() {
        let mut buf = initrd_window();
        region(&mut buf, 0, 0x0010_0000, 0x0001_0000, MemoryKind::KernelAndModules as u32);
        region(&mut buf, 1, 0x0011_0000, 0x000f_0000, MemoryKind::Usable as u32);
        initrd_at(&mut buf, 0x0010_8000, 0x0001_0000);
        stage2_header(&mut buf, 2);
        assert_eq!(
            probe(&buf, BootArch::X86_32).unwrap_err(),
            BootInfoError::InitrdNotReserved
        );
    }

    #[test]
    fn an_initrd_with_no_kernel_and_modules_region_is_refused() {
        let mut buf = initrd_window();
        region(&mut buf, 0, 0x0010_0000, 0x0010_0000, MemoryKind::Usable as u32);
        initrd_at(&mut buf, 0x0010_1000, 0x1000);
        stage2_header(&mut buf, 1);
        assert_eq!(
            probe(&buf, BootArch::X86_32).unwrap_err(),
            BootInfoError::InitrdNotReserved
        );
    }

    #[test]
    fn an_initrd_inside_the_kernel_and_modules_carve_is_the_slice_it_covers() {
        let mut buf = initrd_window();
        let base = (buf.as_ptr() as usize + INITRD_OFFSET - 0x800) as u64;
        kernel_and_modules(&mut buf, base, 0x1000);
        let address = (buf.as_ptr() as usize + INITRD_OFFSET) as u64;
        initrd_at(&mut buf, address, 64);
        stage2_header(&mut buf, 1);
        let slice = probe(&buf, BootArch::X86_32)
            .unwrap()
            .expect("the carve reserves the initrd");
        assert_eq!(slice.len(), 64);
        assert!(slice.iter().all(|byte| *byte == INITRD_FILL));
    }

    #[test]
    fn a_long_mode_initrd_address_translates_to_the_physical_map() {
        let mut buf = initrd_window();
        let physical = (buf.as_ptr() as usize + INITRD_OFFSET - 0x800) as u64;
        kernel_and_modules(&mut buf, physical, 0x1000);
        let wire = (buf.as_ptr() as usize + INITRD_OFFSET) as u64 + HIGHER_HALF_OFFSET;
        initrd_at(&mut buf, wire, 64);
        stage2_long_header(&mut buf, 1);
        let slice = probe(&buf, BootArch::X86_64)
            .unwrap()
            .expect("the higher-half alias resolves to the reserved carve");
        assert_eq!(slice.len(), 64);
        assert_eq!(slice.as_ptr() as u64, wire);

        let mut buf = initrd_window();
        kernel_and_modules(&mut buf, wire, 0x1000);
        initrd_at(&mut buf, wire, 64);
        stage2_long_header(&mut buf, 1);
        assert_eq!(
            probe(&buf, BootArch::X86_64).unwrap_err(),
            BootInfoError::InitrdNotReserved
        );
    }
}
