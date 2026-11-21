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
        blocks.blocks[blocks.count] = MemoryBlock {
            start: start as usize,
            size: (end - start) as usize,
        };
        blocks.count += 1;
        facts.accepted += 1;
    }
    Ok(facts)
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
            blocks: core::array::from_fn(|_| MemoryBlock { start: 0, size: 0 }),
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
}
