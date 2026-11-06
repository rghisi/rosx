#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct MemoryRegion {
    pub start: usize,
    pub size: usize,
}

pub const MAX_BOOT_MEMORY_REGIONS: usize = 32;

#[derive(Copy, Clone)]
pub struct BootMemory {
    regions: [MemoryRegion; MAX_BOOT_MEMORY_REGIONS],
    count: usize,
}

impl BootMemory {
    pub fn regions(&self) -> &[MemoryRegion] {
        &self.regions[..self.count]
    }
}

pub const MAX_BOOT_MODULES: usize = 8;

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct BootModule {
    pub start: usize,
    pub size: usize,
}

#[derive(Copy, Clone)]
pub struct BootModules {
    modules: [BootModule; MAX_BOOT_MODULES],
    count: usize,
}

impl BootModules {
    pub fn list(&self) -> &[BootModule] {
        &self.modules[..self.count]
    }
}

const FLAG_MEMORY_SIZES: u32 = 1 << 0;
const FLAG_MODULES: u32 = 1 << 3;
const FLAG_MEMORY_MAP: u32 = 1 << 6;

const FLAGS_OFFSET: usize = 0;
const MEM_LOWER_OFFSET: usize = 4;
const MEM_UPPER_OFFSET: usize = 8;
const MODS_COUNT_OFFSET: usize = 20;
const MODS_ADDRESS_OFFSET: usize = 24;
const MMAP_LENGTH_OFFSET: usize = 44;
const MMAP_ADDRESS_OFFSET: usize = 48;

const ENTRY_SIZE_OFFSET: usize = 0;
const ENTRY_ADDRESS_OFFSET: usize = 4;
const ENTRY_LENGTH_OFFSET: usize = 12;
const ENTRY_TYPE_OFFSET: usize = 20;
const ENTRY_MIN_BYTES: usize = 24;
const ENTRY_STRIDE_EXTRA: usize = 4;
const ENTRY_AVAILABLE: u32 = 1;

const MODULE_STRIDE_BYTES: usize = 16;
const MODULE_START_OFFSET: usize = 0;
const MODULE_END_OFFSET: usize = 4;

const BYTES_PER_KIB: u64 = 1024;
const ONE_MIB: u64 = 1024 * 1024;
const MAX_ADDRESSABLE: u64 = usize::MAX as u64;

pub unsafe fn usable_memory(info_ptr: *const u8) -> BootMemory {
    let flags = unsafe { read_info_u32(info_ptr, FLAGS_OFFSET) };
    let mem_lower_kib = unsafe { read_info_u32(info_ptr, MEM_LOWER_OFFSET) };
    let mem_upper_kib = unsafe { read_info_u32(info_ptr, MEM_UPPER_OFFSET) };
    let mmap_entries = if flags & FLAG_MEMORY_MAP != 0 {
        let address = unsafe { read_info_u32(info_ptr, MMAP_ADDRESS_OFFSET) } as usize;
        let length = unsafe { read_info_u32(info_ptr, MMAP_LENGTH_OFFSET) } as usize;
        unsafe { core::slice::from_raw_parts(address as *const u8, length) }
    } else {
        &[]
    };
    decode(flags, mem_lower_kib, mem_upper_kib, mmap_entries)
}

fn decode(flags: u32, mem_lower_kib: u32, mem_upper_kib: u32, mmap_entries: &[u8]) -> BootMemory {
    if flags & FLAG_MEMORY_MAP != 0 {
        return mmap_memory(mmap_entries);
    }
    if flags & FLAG_MEMORY_SIZES != 0 {
        return sized_memory(mem_lower_kib, mem_upper_kib);
    }
    empty_memory()
}

pub unsafe fn boot_modules(info_ptr: *const u8) -> BootModules {
    let flags = unsafe { read_info_u32(info_ptr, FLAGS_OFFSET) };
    let mods_count = unsafe { read_info_u32(info_ptr, MODS_COUNT_OFFSET) };
    let records = if flags & FLAG_MODULES != 0 {
        let address = unsafe { read_info_u32(info_ptr, MODS_ADDRESS_OFFSET) } as usize;
        let bounded = core::cmp::min(mods_count, MAX_BOOT_MODULES as u32) as usize;
        unsafe { core::slice::from_raw_parts(address as *const u8, bounded * MODULE_STRIDE_BYTES) }
    } else {
        &[]
    };
    decode_modules(flags, mods_count, records)
}

fn decode_modules(flags: u32, mods_count: u32, records: &[u8]) -> BootModules {
    let mut modules = BootModules {
        modules: core::array::from_fn(|_| BootModule { start: 0, size: 0 }),
        count: 0,
    };
    if flags & FLAG_MODULES == 0 {
        return modules;
    }
    let declared = core::cmp::min(mods_count as usize, MAX_BOOT_MODULES);
    let present = records.len() / MODULE_STRIDE_BYTES;
    for index in 0..core::cmp::min(declared, present) {
        let record = index * MODULE_STRIDE_BYTES;
        let start = read_u32(records, record + MODULE_START_OFFSET) as usize;
        let end = read_u32(records, record + MODULE_END_OFFSET) as usize;
        if start < end {
            modules.modules[modules.count] = BootModule { start, size: end - start };
            modules.count += 1;
        }
    }
    modules
}

fn mmap_memory(entries: &[u8]) -> BootMemory {
    let mut memory = empty_memory();
    let mut offset = 0;
    while offset + ENTRY_MIN_BYTES <= entries.len() && memory.count < MAX_BOOT_MEMORY_REGIONS {
        let size = read_u32(entries, offset + ENTRY_SIZE_OFFSET) as usize;
        let start = read_u64(entries, offset + ENTRY_ADDRESS_OFFSET);
        let length = read_u64(entries, offset + ENTRY_LENGTH_OFFSET);
        if read_u32(entries, offset + ENTRY_TYPE_OFFSET) == ENTRY_AVAILABLE
            && start + length <= MAX_ADDRESSABLE
        {
            memory.regions[memory.count] =
                MemoryRegion { start: start as usize, size: length as usize };
            memory.count += 1;
        }
        offset += size + ENTRY_STRIDE_EXTRA;
    }
    memory
}

fn sized_memory(mem_lower_kib: u32, mem_upper_kib: u32) -> BootMemory {
    let mut memory = empty_memory();
    let lower_end = BYTES_PER_KIB * mem_lower_kib as u64;
    if mem_lower_kib > 0 && lower_end <= MAX_ADDRESSABLE {
        memory.regions[memory.count] = MemoryRegion { start: 0, size: lower_end as usize };
        memory.count += 1;
    }
    let upper_end = ONE_MIB + BYTES_PER_KIB * mem_upper_kib as u64;
    if mem_upper_kib > 0 && upper_end <= MAX_ADDRESSABLE {
        memory.regions[memory.count] =
            MemoryRegion { start: ONE_MIB as usize, size: (upper_end - ONE_MIB) as usize };
        memory.count += 1;
    }
    memory
}

fn empty_memory() -> BootMemory {
    BootMemory {
        regions: core::array::from_fn(|_| MemoryRegion { start: 0, size: 0 }),
        count: 0,
    }
}

fn read_info_u32(base: *const u8, offset: usize) -> u32 {
    unsafe { (base.add(offset) as *const u32).read_unaligned() }
}

fn read_u32(entries: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(entries[offset..offset + 4].try_into().unwrap())
}

fn read_u64(entries: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(entries[offset..offset + 8].try_into().unwrap())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn region_pairs(memory: &BootMemory) -> Vec<(usize, usize)> {
        memory
            .regions()
            .iter()
            .map(|r| (r.start, r.size))
            .collect::<Vec<_>>()
    }

    fn push_u32(buffer: &mut Vec<u8>, value: u32) {
        buffer.extend_from_slice(&value.to_le_bytes());
    }

    fn push_u64(buffer: &mut Vec<u8>, value: u64) {
        buffer.extend_from_slice(&value.to_le_bytes());
    }

    fn mmap_entry(size_field: u32, start: u64, length: u64, entry_type: u32) -> Vec<u8> {
        let mut entry = Vec::new();
        push_u32(&mut entry, size_field);
        push_u64(&mut entry, start);
        push_u64(&mut entry, length);
        push_u32(&mut entry, entry_type);
        entry
    }

    fn standard_entry(start: u64, length: u64, entry_type: u32) -> Vec<u8> {
        mmap_entry(20, start, length, entry_type)
    }

    #[test]
    fn mmap_flag_keeps_only_available_regions() {
        let mut entries = standard_entry(0, 640 * 1024, 1);
        entries.extend(standard_entry(640 * 1024, 128 * 1024, 2));
        entries.extend(standard_entry(1024 * 1024, 16 * 1024 * 1024, 1));
        let memory = decode(FLAG_MEMORY_MAP, 0, 0, &entries);
        assert_eq!(
            region_pairs(&memory),
            vec![(0, 640 * 1024), (1024 * 1024, 16 * 1024 * 1024)]
        );
    }

    #[test]
    fn mmap_entries_stride_by_size_field_plus_four() {
        let mut entries = mmap_entry(28, 0, 1024, 1);
        entries.extend([0u8; 8]);
        entries.extend(standard_entry(1024 * 1024, 2048, 1));
        let memory = decode(FLAG_MEMORY_MAP, 0, 0, &entries);
        assert_eq!(region_pairs(&memory), vec![(0, 1024), (1024 * 1024, 2048)]);
    }

    #[test]
    fn mmap_flag_with_empty_buffer_yields_no_regions() {
        assert_eq!(decode(FLAG_MEMORY_MAP, 640, 16384, &[]).regions().len(), 0);
    }

    #[test]
    fn mmap_buffer_truncated_mid_entry_keeps_complete_regions() {
        let mut entries = standard_entry(0, 4096, 1);
        entries.extend(&standard_entry(4096, 4096, 1)[..16]);
        let memory = decode(FLAG_MEMORY_MAP, 0, 0, &entries);
        assert_eq!(region_pairs(&memory), vec![(0, 4096)]);
    }

    #[test]
    fn mmap_flag_wins_over_memory_sizes() {
        let entries = standard_entry(0, 4096, 1);
        let memory = decode(FLAG_MEMORY_MAP | FLAG_MEMORY_SIZES, 640, 16384, &entries);
        assert_eq!(region_pairs(&memory), vec![(0, 4096)]);
    }

    #[test]
    fn sizes_flag_builds_low_and_extended_regions() {
        let memory = decode(FLAG_MEMORY_SIZES, 640, 16384, &[]);
        assert_eq!(
            region_pairs(&memory),
            vec![(0, 640 * 1024), (ONE_MIB as usize, 16 * 1024 * 1024)]
        );
    }

    #[test]
    fn sizes_flag_drops_zero_sized_regions() {
        assert_eq!(
            region_pairs(&decode(FLAG_MEMORY_SIZES, 640, 0, &[])),
            vec![(0, 640 * 1024)]
        );
        assert_eq!(
            region_pairs(&decode(FLAG_MEMORY_SIZES, 0, 16384, &[])),
            vec![(ONE_MIB as usize, 16 * 1024 * 1024)]
        );
        assert_eq!(decode(FLAG_MEMORY_SIZES, 0, 0, &[]).regions().len(), 0);
    }

    #[test]
    fn absent_flags_yield_no_regions() {
        assert_eq!(decode(0, 640, 16384, &[]).regions().len(), 0);
        assert_eq!(decode(1 << 3, 0, 0, &[]).regions().len(), 0);
    }

    #[test]
    fn malformed_entry_types_are_filtered_from_mmap_walk() {
        let mut entries = standard_entry(0, 4096, 0);
        entries.extend(standard_entry(4096, 4096, 3));
        entries.extend(standard_entry(8192, 4096, 1));
        let memory = decode(FLAG_MEMORY_MAP, 0, 0, &entries);
        assert_eq!(region_pairs(&memory), vec![(8192, 4096)]);
    }

    fn module_record(start: u32, end: u32) -> Vec<u8> {
        let mut record = Vec::new();
        push_u32(&mut record, start);
        push_u32(&mut record, end);
        push_u32(&mut record, 0);
        push_u32(&mut record, 0);
        record
    }

    fn module_pairs(modules: &BootModules) -> Vec<(usize, usize)> {
        modules
            .list()
            .iter()
            .map(|m| (m.start, m.size))
            .collect::<Vec<_>>()
    }

    #[test]
    fn module_field_offsets_match_the_multiboot_spec() {
        assert_eq!(FLAGS_OFFSET, 0);
        assert_eq!(MEM_LOWER_OFFSET, 4);
        assert_eq!(MEM_UPPER_OFFSET, 8);
        assert_eq!(MODS_COUNT_OFFSET, 20);
        assert_eq!(MODS_ADDRESS_OFFSET, 24);
        assert_eq!(MMAP_LENGTH_OFFSET, 44);
        assert_eq!(MMAP_ADDRESS_OFFSET, 48);
    }

    #[test]
    fn modules_flag_keeps_declared_ranges_in_order() {
        let records = [module_record(0x100000, 0x101000), module_record(0x200000, 0x202000)]
            .concat();
        let modules = decode_modules(FLAG_MODULES, 2, &records);
        assert_eq!(module_pairs(&modules), vec![(0x100000, 0x1000), (0x200000, 0x2000)]);
    }

    #[test]
    fn absent_modules_flag_yields_no_modules() {
        let records = module_record(0x100000, 0x101000);
        assert_eq!(decode_modules(0, 1, &records).list().len(), 0);
    }

    #[test]
    fn zero_module_count_yields_no_modules() {
        let records = module_record(0x100000, 0x101000);
        assert_eq!(decode_modules(FLAG_MODULES, 0, &records).list().len(), 0);
    }

    #[test]
    fn declared_count_beyond_present_bytes_keeps_complete_records() {
        let records = module_record(0x100000, 0x101000);
        let modules = decode_modules(FLAG_MODULES, 4, &records);
        assert_eq!(module_pairs(&modules), vec![(0x100000, 0x1000)]);
    }

    #[test]
    fn degenerate_module_ranges_are_skipped() {
        let records = [
            module_record(0x100000, 0x100000),
            module_record(0x201000, 0x200000),
            module_record(0x300000, 0x300010),
        ]
        .concat();
        let modules = decode_modules(FLAG_MODULES, 3, &records);
        assert_eq!(module_pairs(&modules), vec![(0x300000, 0x10)]);
    }

    #[test]
    fn module_count_is_capped_at_the_static_bound() {
        let records: Vec<u8> = (0..(MAX_BOOT_MODULES + 2) as u32)
            .flat_map(|index| module_record(0x100000 + index * 0x1000, 0x100100 + index * 0x1000))
            .collect();
        let modules = decode_modules(FLAG_MODULES, (MAX_BOOT_MODULES + 2) as u32, &records);
        assert_eq!(modules.list().len(), MAX_BOOT_MODULES);
        assert_eq!(modules.list()[0], BootModule { start: 0x100000, size: 0x100 });
    }
}
