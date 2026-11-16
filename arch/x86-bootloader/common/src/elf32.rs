pub const ELF_MAGIC: [u8; 4] = [0x7F, b'E', b'L', b'F'];
pub const ELF_CLASS_32: u8 = 1;
pub const ELF_DATA_LITTLE_ENDIAN: u8 = 1;
pub const EM_386: u16 = 3;
pub const PT_LOAD: u32 = 1;

pub const MIN_KERNEL_ADDRESS: u64 = 0x0010_0000;
pub const PAGE_BYTES: u64 = 4096;
pub const ELF32_HEADER_BYTES: usize = 52;
pub const ELF32_PROGRAM_HEADER_BYTES: usize = 32;
pub const MAX_PROGRAM_HEADERS: usize = 64;
pub const MAX_KERNEL_SEGMENTS: usize = 8;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ElfError {
    Class,
    Machine,
    Format,
    Layout,
    Entry,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct KernelSegment {
    pub file_offset: u64,
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
    memory_address: 0,
    file_bytes: 0,
    memory_bytes: 0,
};

pub fn plan_kernel_image(table: &[u8]) -> Result<KernelImage, ElfError> {
    let header = header_fields(table)?;
    let mut segments = [EMPTY_SEGMENT; MAX_KERNEL_SEGMENTS];
    let mut segment_count = 0usize;
    for index in 0..header.program_count {
        let start = header.table_offset + index * header.program_entry_size;
        let record = table
            .get(start..start + ELF32_PROGRAM_HEADER_BYTES)
            .ok_or(ElfError::Format)?;
        if read_u32(record, 0) != PT_LOAD {
            continue;
        }
        let segment = KernelSegment {
            file_offset: u64::from(read_u32(record, 4)),
            memory_address: u64::from(read_u32(record, 12)),
            file_bytes: u64::from(read_u32(record, 16)),
            memory_bytes: u64::from(read_u32(record, 20)),
        };
        if segment.memory_bytes == 0 {
            continue;
        }
        if segment.memory_address < MIN_KERNEL_ADDRESS || segment.file_bytes > segment.memory_bytes
        {
            return Err(ElfError::Layout);
        }
        if overlaps_loaded(&segments[..segment_count], segment) {
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
        .any(|segment| inside_span(header.entry, segment))
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
        entry: header.entry,
        segments,
        segment_count,
        phys_start,
        phys_end,
    })
}

struct HeaderFields {
    entry: u64,
    table_offset: usize,
    program_entry_size: usize,
    program_count: usize,
}

fn header_fields(table: &[u8]) -> Result<HeaderFields, ElfError> {
    let header = table.get(..ELF32_HEADER_BYTES).ok_or(ElfError::Format)?;
    if header[..ELF_MAGIC.len()] != ELF_MAGIC {
        return Err(ElfError::Format);
    }
    if header[4] != ELF_CLASS_32 || header[5] != ELF_DATA_LITTLE_ENDIAN {
        return Err(ElfError::Class);
    }
    if read_u16(header, 18) != EM_386 {
        return Err(ElfError::Machine);
    }
    let table_offset = read_u32(header, 28) as usize;
    let program_entry_size = read_u16(header, 42) as usize;
    let program_count = read_u16(header, 44) as usize;
    if table_offset < ELF32_HEADER_BYTES
        || program_entry_size < ELF32_PROGRAM_HEADER_BYTES
        || program_count > MAX_PROGRAM_HEADERS
    {
        return Err(ElfError::Format);
    }
    Ok(HeaderFields {
        entry: u64::from(read_u32(header, 24)),
        table_offset,
        program_entry_size,
        program_count,
    })
}

fn inside_span(address: u64, segment: &KernelSegment) -> bool {
    address >= segment.memory_address
        && address < segment.memory_address.saturating_add(segment.memory_bytes)
}

fn overlaps_loaded(loaded: &[KernelSegment], candidate: KernelSegment) -> bool {
    loaded.iter().any(|segment| {
        let candidate_end = candidate
            .memory_address
            .saturating_add(candidate.memory_bytes);
        let segment_end = segment
            .memory_address
            .saturating_add(segment.memory_bytes);
        candidate.memory_address < segment_end && segment.memory_address < candidate_end
    })
}

fn align_up_page(bytes: u64) -> u64 {
    bytes.saturating_add(PAGE_BYTES - 1) & !(PAGE_BYTES - 1)
}

fn read_u16(bytes: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([bytes[at], bytes[at + 1]])
}

fn read_u32(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}

#[cfg(test)]
mod tests {
    use super::*;

    extern crate alloc;
    use alloc::vec;
    use alloc::vec::Vec;

    fn header_bytes(class: u8, data: u8, machine: u16, entry: u32, count: u16) -> Vec<u8> {
        let mut header = vec![0u8; ELF32_HEADER_BYTES];
        header[..4].copy_from_slice(&ELF_MAGIC);
        header[4] = class;
        header[5] = data;
        header[18..20].copy_from_slice(&machine.to_le_bytes());
        header[24..28].copy_from_slice(&entry.to_le_bytes());
        header[28..32].copy_from_slice(&(ELF32_HEADER_BYTES as u32).to_le_bytes());
        header[42..44].copy_from_slice(&(ELF32_PROGRAM_HEADER_BYTES as u16).to_le_bytes());
        header[44..46].copy_from_slice(&count.to_le_bytes());
        header
    }

    fn load_program(file_offset: u32, memory_address: u32, file_bytes: u32, memory_bytes: u32) -> Vec<u8> {
        let mut record = vec![0u8; ELF32_PROGRAM_HEADER_BYTES];
        record[0..4].copy_from_slice(&PT_LOAD.to_le_bytes());
        record[4..8].copy_from_slice(&file_offset.to_le_bytes());
        record[12..16].copy_from_slice(&memory_address.to_le_bytes());
        record[16..20].copy_from_slice(&file_bytes.to_le_bytes());
        record[20..24].copy_from_slice(&memory_bytes.to_le_bytes());
        record
    }

    fn image_table(programs: &[&[u8]]) -> Vec<u8> {
        let mut table = header_bytes(ELF_CLASS_32, ELF_DATA_LITTLE_ENDIAN, EM_386, 0x0010_0000, programs.len() as u16);
        for program in programs {
            table.extend_from_slice(program);
        }
        table
    }

    #[test]
    fn a_minimal_em386_load_image_plans_the_kernel() {
        let table = image_table(&[&load_program(0x1000, 0x0010_0000, 4096, 4096)]);
        let image = plan_kernel_image(&table).expect("minimal image plans");
        assert_eq!(image.entry, 0x0010_0000);
        assert_eq!(image.segment_count, 1);
        assert_eq!(image.segments[0].file_offset, 0x1000);
        assert_eq!(image.phys_start, 0x0010_0000);
        assert_eq!(image.phys_end, 0x0010_1000);
    }

    #[test]
    fn a_64_bit_class_is_refused_as_class() {
        let mut table = image_table(&[&load_program(0, 0x0010_0000, 4096, 4096)]);
        table[4] = 2;
        assert_eq!(plan_kernel_image(&table), Err(ElfError::Class));
    }

    #[test]
    fn a_big_endian_image_is_refused_as_class() {
        let mut table = image_table(&[&load_program(0, 0x0010_0000, 4096, 4096)]);
        table[5] = 2;
        assert_eq!(plan_kernel_image(&table), Err(ElfError::Class));
    }

    #[test]
    fn a_foreign_machine_is_refused_as_machine() {
        let mut table = image_table(&[]);
        let x86_64_machine = 0x003Eu16;
        table[18..20].copy_from_slice(&x86_64_machine.to_le_bytes());
        assert_eq!(plan_kernel_image(&table), Err(ElfError::Machine));
    }

    #[test]
    fn a_missing_magic_is_refused_as_format() {
        let mut table = image_table(&[]);
        table[1] = b'X';
        assert_eq!(plan_kernel_image(&table), Err(ElfError::Format));
    }

    #[test]
    fn a_truncated_header_is_refused_as_format() {
        let table = vec![0u8; ELF32_HEADER_BYTES - 1];
        assert_eq!(plan_kernel_image(&table), Err(ElfError::Format));
    }

    #[test]
    fn a_program_table_running_past_the_bytes_is_refused_as_format() {
        let table = image_table(&[&load_program(0, 0x0010_0000, 4096, 4096)]);
        assert_eq!(
            plan_kernel_image(&table[..ELF32_HEADER_BYTES + ELF32_PROGRAM_HEADER_BYTES - 1]),
            Err(ElfError::Format)
        );
    }

    #[test]
    fn a_program_entry_size_below_the_elf32_layout_is_refused_as_format() {
        let mut table = image_table(&[]);
        table[42..44].copy_from_slice(&31u16.to_le_bytes());
        assert_eq!(plan_kernel_image(&table), Err(ElfError::Format));
    }

    #[test]
    fn a_program_table_claiming_to_start_inside_the_header_is_refused_as_format() {
        let mut table = image_table(&[]);
        table[28..32].copy_from_slice(&(ELF32_HEADER_BYTES as u32 - 1).to_le_bytes());
        assert_eq!(plan_kernel_image(&table), Err(ElfError::Format));
    }

    #[test]
    fn an_empty_program_table_is_refused_as_layout() {
        let table = image_table(&[]);
        assert_eq!(plan_kernel_image(&table), Err(ElfError::Layout));
    }

    #[test]
    fn a_load_segment_below_the_first_mebibyte_is_refused_as_layout() {
        for address in [0u32, 0x0009_FFFF] {
            let table = image_table(&[&load_program(0, address, 512, 512)]);
            assert_eq!(plan_kernel_image(&table), Err(ElfError::Layout));
        }
    }

    #[test]
    fn a_segment_at_exactly_the_first_mebibyte_is_accepted() {
        let table = image_table(&[&load_program(0, 0x0010_0000, 512, 512)]);
        assert!(plan_kernel_image(&table).is_ok());
    }

    #[test]
    fn overlapping_load_segments_are_refused_as_layout() {
        let table = image_table(&[
            &load_program(0, 0x0010_0000, 0x2000, 0x2000),
            &load_program(0x2000, 0x0010_1000, 0x1000, 0x1000),
        ]);
        assert_eq!(plan_kernel_image(&table), Err(ElfError::Layout));
    }

    #[test]
    fn touching_load_segments_do_not_overlap() {
        let table = image_table(&[
            &load_program(0, 0x0010_0000, 0x1000, 0x1000),
            &load_program(0x1000, 0x0010_1000, 0x1000, 0x1000),
        ]);
        let image = plan_kernel_image(&table).expect("touching spans coexist");
        assert_eq!(image.segment_count, 2);
        assert_eq!(image.phys_end, 0x0010_2000);
    }

    #[test]
    fn a_segment_claiming_more_file_than_memory_is_refused_as_layout() {
        let table = image_table(&[&load_program(0, 0x0010_0000, 0x2000, 0x1000)]);
        assert_eq!(plan_kernel_image(&table), Err(ElfError::Layout));
    }

    #[test]
    fn phys_end_aligns_up_to_the_next_page() {
        let table = image_table(&[&load_program(0, 0x0010_0000, 100, 5000)]);
        let image = plan_kernel_image(&table).expect("unaligned tail plans");
        assert_eq!(image.phys_end, 0x0010_2000);
    }

    #[test]
    fn phys_start_is_the_lowest_address_of_an_unsorted_table() {
        let mut table = header_bytes(ELF_CLASS_32, ELF_DATA_LITTLE_ENDIAN, EM_386, 0x0010_9000, 2);
        table.extend_from_slice(&load_program(0x2000, 0x0010_9000, 0x1000, 0x1000));
        table.extend_from_slice(&load_program(0, 0x0010_0000, 0x1000, 0x1000));
        let image = plan_kernel_image(&table).expect("unsorted table plans");
        assert_eq!(image.phys_start, 0x0010_0000);
        assert_eq!(image.phys_end, 0x0010_A000);
    }

    #[test]
    fn an_entry_outside_every_loaded_span_is_refused_as_entry() {
        let mut table = header_bytes(ELF_CLASS_32, ELF_DATA_LITTLE_ENDIAN, EM_386, 0x0020_0000, 1);
        table.extend_from_slice(&load_program(0, 0x0010_0000, 0x1000, 0x1000));
        assert_eq!(plan_kernel_image(&table), Err(ElfError::Entry));
    }

    #[test]
    fn an_entry_at_the_end_of_a_span_is_outside_it() {
        let mut table = header_bytes(ELF_CLASS_32, ELF_DATA_LITTLE_ENDIAN, EM_386, 0x0010_1000, 1);
        table.extend_from_slice(&load_program(0, 0x0010_0000, 0x1000, 0x1000));
        assert_eq!(plan_kernel_image(&table), Err(ElfError::Entry));
    }

    #[test]
    fn an_entry_inside_the_zero_fill_tail_is_accepted() {
        let mut table = header_bytes(ELF_CLASS_32, ELF_DATA_LITTLE_ENDIAN, EM_386, 0x0010_1800, 1);
        table.extend_from_slice(&load_program(0, 0x0010_0000, 0x1000, 0x2000));
        assert!(plan_kernel_image(&table).is_ok());
    }

    #[test]
    fn a_load_segment_without_memory_is_skipped() {
        let table = image_table(&[
            &load_program(0, 0x0010_0000, 0x1000, 0x1000),
            &load_program(0x1000, 0x0010_8000, 0, 0),
        ]);
        let image = plan_kernel_image(&table).expect("empty segment is not a segment");
        assert_eq!(image.segment_count, 1);
    }

    #[test]
    fn non_load_segments_are_skipped() {
        let mut interpreter = vec![0u8; ELF32_PROGRAM_HEADER_BYTES];
        interpreter[0..4].copy_from_slice(&2u32.to_le_bytes());
        let table = image_table(&[
            &interpreter,
            &load_program(0, 0x0010_0000, 0x1000, 0x1000),
        ]);
        let image = plan_kernel_image(&table).expect("PT_INTERP is not a load segment");
        assert_eq!(image.segment_count, 1);
    }

    #[test]
    fn more_segments_than_the_array_holds_are_refused_as_layout() {
        let mut table = header_bytes(ELF_CLASS_32, ELF_DATA_LITTLE_ENDIAN, EM_386, 0x0010_0000, 9);
        for index in 0..9u64 {
            table.extend_from_slice(&load_program(
                0,
                (0x0010_0000 + index * 0x1000) as u32,
                0x1000,
                0x1000,
            ));
        }
        assert_eq!(plan_kernel_image(&table), Err(ElfError::Layout));
    }

    #[test]
    fn more_program_headers_than_the_guard_are_refused_as_format() {
        let mut table = image_table(&[]);
        table[44..46].copy_from_slice(&((MAX_PROGRAM_HEADERS + 1) as u16).to_le_bytes());
        assert_eq!(plan_kernel_image(&table), Err(ElfError::Format));
    }
}
