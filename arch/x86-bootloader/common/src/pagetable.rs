pub const ENTRIES_PER_TABLE: usize = 512;
pub const PAGE_TABLE_BYTES: usize = 4096;
pub const PAGE_TABLE_SET_BYTES: usize = 6 * PAGE_TABLE_BYTES;
pub const PRESENT: u64 = 0b01;
pub const WRITABLE: u64 = 0b10;
pub const HUGE_PAGE_2MIB: u64 = 1 << 5;
pub const HIGHER_HALF_PML4_INDEX: usize = 256;
pub const TABLES_PER_SET: usize = 6;

type PageTable = [u64; ENTRIES_PER_TABLE];

pub const fn huge_page(two_mib_page: u32) -> u64 {
    ((two_mib_page as u64) << 21) | PRESENT | WRITABLE | HUGE_PAGE_2MIB
}

pub const fn next_table(table_address: u64) -> u64 {
    table_address | PRESENT | WRITABLE
}

pub fn fill_identity_map(set: &mut [PageTable; TABLES_PER_SET], base: u64) {
    for table in set.iter_mut() {
        for entry in table.iter_mut() {
            *entry = 0;
        }
    }
    for (directory_index, directory) in set[2..].iter_mut().enumerate() {
        for (entry_index, entry) in directory.iter_mut().enumerate() {
            *entry = huge_page((directory_index * ENTRIES_PER_TABLE + entry_index) as u32);
        }
    }
    for (directory_index, entry) in set[1][..4].iter_mut().enumerate() {
        *entry = next_table(base + (2 + directory_index) as u64 * PAGE_TABLE_BYTES as u64);
    }
    set[0][0] = next_table(base + PAGE_TABLE_BYTES as u64);
    set[0][HIGHER_HALF_PML4_INDEX] = next_table(base + PAGE_TABLE_BYTES as u64);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_set() -> [PageTable; TABLES_PER_SET] {
        [[0u64; ENTRIES_PER_TABLE]; TABLES_PER_SET]
    }

    fn filled(base: u64) -> [PageTable; TABLES_PER_SET] {
        let mut set = empty_set();
        fill_identity_map(&mut set, base);
        set
    }

    #[test]
    fn huge_pages_carry_present_writable_and_page_size() {
        assert_eq!(huge_page(0), 0x23);
        assert_eq!(huge_page(1), 0x200_023);
        assert_eq!(huge_page(2047), 0xFFE0_0023);
    }

    #[test]
    fn table_pointers_carry_present_and_writable_only() {
        assert_eq!(next_table(0x0011_1000), 0x0011_1003);
    }

    #[test]
    fn the_four_page_directories_map_two_gib_of_two_mib_pages() {
        let set = filled(0x0100_0000);
        for (directory_index, directory) in set[2..].iter().enumerate() {
            for (entry_index, entry) in directory.iter().enumerate() {
                let expected_page = (directory_index * 512 + entry_index) as u64;
                assert_eq!(*entry & !0xFFF, expected_page << 21);
                assert_eq!(*entry & 0xFFF, PRESENT | WRITABLE | HUGE_PAGE_2MIB);
            }
        }
    }

    #[test]
    fn the_identity_map_reaches_four_gib() {
        let set = filled(0x0100_0000);
        assert_eq!(set[5][511] & !0xFFF, 0xFFE0_0000);
    }

    #[test]
    fn the_page_directory_pointer_table_names_the_four_directories() {
        let base = 0x0100_0000;
        let set = filled(base);
        for (directory_index, entry) in set[1][..4].iter().enumerate() {
            let expected = base + (2 + directory_index) as u64 * PAGE_TABLE_BYTES as u64;
            assert_eq!(*entry, expected | PRESENT | WRITABLE);
        }
        for entry in &set[1][4..] {
            assert_eq!(*entry, 0);
        }
    }

    #[test]
    fn both_pml4_windows_alias_the_same_page_directory_pointer_table() {
        let base = 0x0100_0000;
        let set = filled(base);
        let pdpt = base + PAGE_TABLE_BYTES as u64;
        assert_eq!(set[0][0], pdpt | PRESENT | WRITABLE);
        assert_eq!(
            set[0][HIGHER_HALF_PML4_INDEX],
            pdpt | PRESENT | WRITABLE
        );
    }

    #[test]
    fn every_other_top_level_entry_stays_absent() {
        let set = filled(0x0100_0000);
        for (index, entry) in set[0].iter().enumerate() {
            if index != 0 && index != HIGHER_HALF_PML4_INDEX {
                assert_eq!(*entry, 0, "PML4 entry {index}");
            }
        }
    }

    #[test]
    fn the_fill_zeroes_a_dirty_set() {
        let mut set = [[0xDEAD_BEEF_CAFE_F00Du64; ENTRIES_PER_TABLE]; TABLES_PER_SET];
        fill_identity_map(&mut set, 0x0100_0000);
        assert_eq!(set, filled(0x0100_0000));
    }
}
