use crate::ElfError;

use crate::elf32::ELF_DATA_LITTLE_ENDIAN;
use crate::elf32::ELF_MAGIC;
use crate::elf32::MAX_KERNEL_SEGMENTS;
use crate::elf32::MAX_PROGRAM_HEADERS;
use crate::elf32::MIN_KERNEL_ADDRESS;
use crate::elf32::PT_LOAD;
use crate::elf32::align_up_page;

pub const ELF_CLASS_64: u8 = 2;
pub const EM_X86_64: u16 = 62;
pub const ELF64_HEADER_BYTES: usize = 64;
pub const ELF64_PROGRAM_HEADER_BYTES: usize = 56;

pub const HIGHER_HALF_TOP_BITS: u64 = 0xFFFF;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct KernelSegment {
    pub file_offset: u64,
    pub virtual_address: u64,
    pub memory_address: u64,
    pub file_bytes: u64,
    pub memory_bytes: u64,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct KernelImage {
    pub entry: u64,
    pub segments: [KernelSegment; MAX_KERNEL_SEGMENTS],
    pub segment_count: usize,
    pub phys_start: u64,
    pub phys_end: u64,
}

const EMPTY_SEGMENT: KernelSegment = KernelSegment {
    file_offset: 0,
    virtual_address: 0,
    memory_address: 0,
    file_bytes: 0,
    memory_bytes: 0,
};

pub fn plan_kernel_image(table: &[u8]) -> Result<KernelImage, ElfError> {
    let header = table
        .get(..ELF64_HEADER_BYTES)
        .ok_or(ElfError::Format)?;
    if header[..ELF_MAGIC.len()] != ELF_MAGIC {
        return Err(ElfError::Format);
    }
    if header[4] != ELF_CLASS_64 || header[5] != ELF_DATA_LITTLE_ENDIAN {
        return Err(ElfError::Class);
    }
    if read_u16(header, 18) != EM_X86_64 {
        return Err(ElfError::Machine);
    }
    let entry = read_u64(header, 24);
    if entry >> 48 != HIGHER_HALF_TOP_BITS {
        return Err(ElfError::Entry);
    }
    let table_offset = read_u64(header, 32) as usize;
    let program_entry_size = read_u16(header, 54) as usize;
    let program_count = read_u16(header, 58) as usize;
    if table_offset < ELF64_HEADER_BYTES
        || program_entry_size < ELF64_PROGRAM_HEADER_BYTES
        || program_count > MAX_PROGRAM_HEADERS
    {
        return Err(ElfError::Format);
    }
    let mut segments = [EMPTY_SEGMENT; MAX_KERNEL_SEGMENTS];
    let mut segment_count = 0usize;
    for index in 0..program_count {
        let start = table_offset + index * program_entry_size;
        let record = table
            .get(start..start + ELF64_PROGRAM_HEADER_BYTES)
            .ok_or(ElfError::Format)?;
        if read_u32(record, 0) != PT_LOAD {
            continue;
        }
        let segment = KernelSegment {
            file_offset: read_u64(record, 8),
            virtual_address: read_u64(record, 16),
            memory_address: read_u64(record, 24),
            file_bytes: read_u64(record, 32),
            memory_bytes: read_u64(record, 40),
        };
        if segment.memory_bytes == 0 {
            continue;
        }
        if segment.memory_address < MIN_KERNEL_ADDRESS
            || segment.file_bytes > segment.memory_bytes
        {
            return Err(ElfError::Layout);
        }
        if overlaps(
            &segments[..segment_count],
            segment.memory_address,
            segment.memory_bytes,
        ) {
            return Err(ElfError::Layout);
        }
        *segments
            .get_mut(segment_count)
            .ok_or(ElfError::Layout)? = segment;
        segment_count += 1;
    }
    if segment_count == 0 {
        return Err(ElfError::Layout);
    }
    if !segments[..segment_count]
        .iter()
        .any(|segment| inside_virtual_span(entry, segment))
    {
        return Err(ElfError::Entry);
    }
    let phys_start = segments[..segment_count]
        .iter()
        .map(|segment| segment.memory_address)
        .min()
        .unwrap_or(MIN_KERNEL_ADDRESS);
    let phys_end = align_up_page(
        segments[..segment_count]
            .iter()
            .map(|segment| segment.memory_address + segment.memory_bytes)
            .max()
            .unwrap_or(0),
    );
    Ok(KernelImage {
        entry,
        segments,
        segment_count,
        phys_start,
        phys_end,
    })
}

fn inside_virtual_span(entry: u64, segment: &KernelSegment) -> bool {
    entry >= segment.virtual_address
        && entry < segment.virtual_address.saturating_add(segment.memory_bytes)
}

fn overlaps(loaded: &[KernelSegment], address: u64, length: u64) -> bool {
    loaded.iter().any(|segment| {
        let candidate_end = address.saturating_add(length);
        let segment_end = segment
            .memory_address
            .saturating_add(segment.memory_bytes);
        candidate_end > segment.memory_address && address < segment_end
    })
}

fn read_u16(bytes: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([bytes[at], bytes[at + 1]])
}

fn read_u32(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}

fn read_u64(bytes: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap())
}

#[cfg(test)]
mod tests {
    use super::*;

    extern crate alloc;
    use alloc::vec;
    use alloc::vec::Vec;
    use system::boot_info::HIGHER_HALF_OFFSET;

    const ENTRY: u64 = 0xFFFF_8000_0010_0000;

    fn header_bytes(class: u8, data: u8, machine: u16, entry: u64, count: u16) -> Vec<u8> {
        let mut header = vec![0u8; ELF64_HEADER_BYTES];
        header[..4].copy_from_slice(&ELF_MAGIC);
        header[4] = class;
        header[5] = data;
        header[18..20].copy_from_slice(&machine.to_le_bytes());
        header[24..32].copy_from_slice(&entry.to_le_bytes());
        header[32..40].copy_from_slice(&(ELF64_HEADER_BYTES as u64).to_le_bytes());
        header[54..56].copy_from_slice(&(ELF64_PROGRAM_HEADER_BYTES as u16).to_le_bytes());
        header[58..60].copy_from_slice(&count.to_le_bytes());
        header
    }

    fn load_program(
        file_offset: u64,
        virtual_address: u64,
        physical_address: u64,
        file_bytes: u64,
        memory_bytes: u64,
    ) -> Vec<u8> {
        let mut record = vec![0u8; ELF64_PROGRAM_HEADER_BYTES];
        record[0..4].copy_from_slice(&PT_LOAD.to_le_bytes());
        record[8..16].copy_from_slice(&file_offset.to_le_bytes());
        record[16..24].copy_from_slice(&virtual_address.to_le_bytes());
        record[24..32].copy_from_slice(&physical_address.to_le_bytes());
        record[32..40].copy_from_slice(&file_bytes.to_le_bytes());
        record[40..48].copy_from_slice(&memory_bytes.to_le_bytes());
        record
    }

    fn segment64(physical: u64, file_bytes: u64, memory_bytes: u64) -> Vec<u8> {
        load_program(
            0x1000,
            physical + HIGHER_HALF_OFFSET,
            physical,
            file_bytes,
            memory_bytes,
        )
    }

    fn image_table(programs: &[&[u8]]) -> Vec<u8> {
        let mut table = header_bytes(ELF_CLASS_64, ELF_DATA_LITTLE_ENDIAN, EM_X86_64, ENTRY, programs.len() as u16);
        for program in programs {
            table.extend_from_slice(program);
        }
        table
    }

    #[test]
    fn a_higher_half_emx86_64_image_plans_at_its_physical_addresses() {
        let table = image_table(&[&segment64(0x0010_0000, 0x1000, 0x2000)]);
        let image = plan_kernel_image(&table).expect("higher-half image plans");
        assert_eq!(image.entry, ENTRY);
        assert_eq!(image.segment_count, 1);
        assert_eq!(image.segments[0].virtual_address, ENTRY);
        assert_eq!(image.segments[0].memory_address, 0x0010_0000);
        assert_eq!(image.phys_start, 0x0010_0000);
        assert_eq!(image.phys_end, 0x0010_2000);
    }

    #[test]
    fn the_elf32_planner_refuses_what_the_elf64_planner_accepts() {
        let table = image_table(&[&segment64(0x0010_0000, 0x1000, 0x2000)]);
        assert_eq!(
            crate::elf32::plan_kernel_image(&table),
            Err(ElfError::Class)
        );
    }

    #[test]
    fn this_planner_refuses_the_elf32_class() {
        let mut table = image_table(&[&segment64(0x0010_0000, 0x1000, 0x1000)]);
        table[4] = crate::elf32::ELF_CLASS_32;
        assert_eq!(plan_kernel_image(&table), Err(ElfError::Class));
    }

    #[test]
    fn a_foreign_machine_is_refused_as_machine() {
        let mut table = image_table(&[&segment64(0x0010_0000, 0x1000, 0x1000)]);
        table[18..20].copy_from_slice(&3u16.to_le_bytes());
        assert_eq!(plan_kernel_image(&table), Err(ElfError::Machine));
    }

    #[test]
    fn an_entry_without_the_higher_half_top_bits_is_refused_as_entry() {
        for entry in [0x0010_0000u64, 0x7FFF_FFFF_FFFF_FFFF, 0xFFFE_8000_0010_0000] {
            let mut table = header_bytes(
                ELF_CLASS_64,
                ELF_DATA_LITTLE_ENDIAN,
                EM_X86_64,
                entry,
                1,
            );
            table.extend_from_slice(&segment64(0x0010_0000, 0x1000, 0x1000));
            assert_eq!(plan_kernel_image(&table), Err(ElfError::Entry), "{entry:#x}");
        }
    }

    #[test]
    fn an_entry_outside_every_virtual_span_is_refused_as_entry() {
        let mut table = header_bytes(
            ELF_CLASS_64,
            ELF_DATA_LITTLE_ENDIAN,
            EM_X86_64,
            0xFFFF_8000_0020_0000,
            1,
        );
        table.extend_from_slice(&segment64(0x0010_0000, 0x1000, 0x1000));
        assert_eq!(plan_kernel_image(&table), Err(ElfError::Entry));
    }

    #[test]
    fn the_entry_may_live_in_the_zero_fill_tail_of_its_span() {
        let mut table = header_bytes(
            ELF_CLASS_64,
            ELF_DATA_LITTLE_ENDIAN,
            EM_X86_64,
            0xFFFF_8000_0010_1800,
            1,
        );
        table.extend_from_slice(&segment64(0x0010_0000, 0x1000, 0x2000));
        assert!(plan_kernel_image(&table).is_ok());
    }

    #[test]
    fn a_load_segment_below_the_first_mebibyte_is_refused_as_layout() {
        let table = image_table(&[&segment64(0x000F_F000, 0x100, 0x100)]);
        assert_eq!(plan_kernel_image(&table), Err(ElfError::Layout));
    }

    #[test]
    fn overlapping_physical_segments_are_refused_as_layout() {
        let table = image_table(&[
            &segment64(0x0010_0000, 0x2000, 0x2000),
            &segment64(0x0010_1000, 0x1000, 0x1000),
        ]);
        assert_eq!(plan_kernel_image(&table), Err(ElfError::Layout));
    }

    #[test]
    fn non_load_segments_are_skipped() {
        let mut note = vec![0u8; ELF64_PROGRAM_HEADER_BYTES];
        note[0..4].copy_from_slice(&4u32.to_le_bytes());
        let table = image_table(&[&note, &segment64(0x0010_0000, 0x1000, 0x1000)]);
        let image = plan_kernel_image(&table).expect("PT_NOTE is not a load segment");
        assert_eq!(image.segment_count, 1);
    }

    #[test]
    fn a_truncated_header_is_refused_as_format() {
        let table = vec![0u8; ELF64_HEADER_BYTES - 1];
        assert_eq!(plan_kernel_image(&table), Err(ElfError::Format));
    }

    #[test]
    fn a_program_entry_size_below_the_elf64_layout_is_refused_as_format() {
        let mut table = image_table(&[&segment64(0x0010_0000, 0x1000, 0x1000)]);
        table[54..56].copy_from_slice(&55u16.to_le_bytes());
        assert_eq!(plan_kernel_image(&table), Err(ElfError::Format));
    }

    #[test]
    fn an_empty_program_table_is_refused_as_layout() {
        let table = image_table(&[]);
        assert_eq!(plan_kernel_image(&table), Err(ElfError::Layout));
    }
}
