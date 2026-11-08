pub const SECTOR_BYTES: usize = 512;
pub const TABLE_OFFSET: usize = 446;
pub const ENTRY_BYTES: usize = 16;
pub const ENTRY_COUNT: usize = 4;
pub const SIGNATURE: [u8; 2] = [0x55, 0xAA];

const SIGNATURE_OFFSET: usize = SECTOR_BYTES - 2;

pub type Slot = u8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Entry {
    pub boot: u8,
    pub type_id: u8,
    pub start_lba: u32,
    pub sector_count: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TableState {
    NoSignature,
    Empty,
    Valid,
    Corrupt,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Table {
    pub state: TableState,
    pub entries: [Option<Entry>; ENTRY_COUNT],
}

pub fn parse_mbr(sector: &[u8; SECTOR_BYTES], disk_sectors: u64) -> Table {
    if !signature_present(sector) {
        return Table {
            state: TableState::NoSignature,
            entries: [None; ENTRY_COUNT],
        };
    }
    if table_bytes(sector).iter().all(|&byte| byte == 0) {
        return Table {
            state: TableState::Empty,
            entries: [None; ENTRY_COUNT],
        };
    }
    let entries = parse_entries(sector);
    let state = if entries_fit_the_disk(&entries, disk_sectors) && entries_are_disjoint(&entries) {
        TableState::Valid
    } else {
        TableState::Corrupt
    };
    Table { state, entries }
}

fn signature_present(sector: &[u8; SECTOR_BYTES]) -> bool {
    sector[SIGNATURE_OFFSET..SECTOR_BYTES] == SIGNATURE
}

fn table_bytes(sector: &[u8; SECTOR_BYTES]) -> &[u8] {
    &sector[TABLE_OFFSET..TABLE_OFFSET + ENTRY_BYTES * ENTRY_COUNT]
}

fn parse_entries(sector: &[u8; SECTOR_BYTES]) -> [Option<Entry>; ENTRY_COUNT] {
    core::array::from_fn(|slot| {
        parse_entry(&table_bytes(sector)[slot * ENTRY_BYTES..(slot + 1) * ENTRY_BYTES])
    })
}

fn parse_entry(bytes: &[u8]) -> Option<Entry> {
    let type_id = bytes[4];
    if type_id == 0 {
        return None;
    }
    Some(Entry {
        boot: bytes[0],
        type_id,
        start_lba: u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]),
        sector_count: u32::from_le_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]),
    })
}

fn entries_fit_the_disk(entries: &[Option<Entry>; ENTRY_COUNT], disk_sectors: u64) -> bool {
    entries.iter().flatten().all(|entry| {
        entry.sector_count > 0
            && entry.start_lba >= 1
            && u64::from(entry.start_lba) + u64::from(entry.sector_count) <= disk_sectors
    })
}

fn entries_are_disjoint(entries: &[Option<Entry>; ENTRY_COUNT]) -> bool {
    for (first_slot, first) in entries.iter().enumerate() {
        let Some(first) = first else { continue };
        for second in entries[first_slot + 1..].iter().flatten() {
            if ranges_overlap(first, second) {
                return false;
            }
        }
    }
    true
}

fn ranges_overlap(first: &Entry, second: &Entry) -> bool {
    let first_start = u64::from(first.start_lba);
    let first_end = first_start + u64::from(first.sector_count);
    let second_start = u64::from(second.start_lba);
    let second_end = second_start + u64::from(second.sector_count);
    first_start < second_end && second_start < first_end
}

#[cfg(test)]
mod tests {
    use super::*;

    const DISK_SECTORS: u64 = 204_800;

    fn signed_master_boot_record() -> [u8; SECTOR_BYTES] {
        let mut sector = [0u8; SECTOR_BYTES];
        sector[SIGNATURE_OFFSET] = SIGNATURE[0];
        sector[SIGNATURE_OFFSET + 1] = SIGNATURE[1];
        sector
    }

    fn stamp_entry(
        sector: &mut [u8; SECTOR_BYTES],
        slot: usize,
        boot: u8,
        type_id: u8,
        start_lba: u32,
        sector_count: u32,
    ) {
        let entry = &mut sector[TABLE_OFFSET + slot * ENTRY_BYTES..][..ENTRY_BYTES];
        entry[0] = boot;
        entry[4] = type_id;
        entry[8..12].copy_from_slice(&start_lba.to_le_bytes());
        entry[12..16].copy_from_slice(&sector_count.to_le_bytes());
    }

    #[test]
    fn canonical_single_linux_partition_at_lba_2048_is_valid() {
        let mut sector = signed_master_boot_record();
        stamp_entry(&mut sector, 0, 0x80, 0x83, 2048, 196_608);
        let table = parse_mbr(&sector, DISK_SECTORS);
        assert_eq!(table.state, TableState::Valid);
        assert_eq!(
            table.entries[0],
            Some(Entry {
                boot: 0x80,
                type_id: 0x83,
                start_lba: 2048,
                sector_count: 196_608,
            })
        );
        assert_eq!(table.entries[1..], [None; ENTRY_COUNT - 1]);
    }

    #[test]
    fn legacy_63_sector_layout_is_valid() {
        let mut sector = signed_master_boot_record();
        stamp_entry(&mut sector, 0, 0x80, 0x06, 63, 1_985);
        let table = parse_mbr(&sector, 2_048);
        assert_eq!(table.state, TableState::Valid);
        assert_eq!(table.entries[0].unwrap().start_lba, 63);
    }

    #[test]
    fn four_in_use_slots_are_all_reported() {
        let mut sector = signed_master_boot_record();
        stamp_entry(&mut sector, 0, 0x00, 0x0c, 2048, 51_200);
        stamp_entry(&mut sector, 1, 0x00, 0x82, 53_248, 51_200);
        stamp_entry(&mut sector, 2, 0x00, 0x83, 104_448, 51_200);
        stamp_entry(&mut sector, 3, 0x00, 0x8e, 155_648, 49_152);
        let table = parse_mbr(&sector, DISK_SECTORS);
        assert_eq!(table.state, TableState::Valid);
        assert!(table.entries.iter().all(Option::is_some));
    }

    #[test]
    fn holes_keep_the_true_slot_indices() {
        let mut sector = signed_master_boot_record();
        stamp_entry(&mut sector, 0, 0x80, 0x0b, 2048, 102_400);
        stamp_entry(&mut sector, 3, 0x00, 0x83, 104_448, 100_352);
        let table = parse_mbr(&sector, DISK_SECTORS);
        assert_eq!(table.state, TableState::Valid);
        assert!(table.entries[0].is_some());
        assert_eq!(table.entries[1..3], [None, None]);
        assert!(table.entries[3].is_some());
    }

    #[test]
    fn boot_indicator_and_garbage_chs_bytes_do_not_change_the_verdict() {
        let mut sector = signed_master_boot_record();
        stamp_entry(&mut sector, 0, 0x80, 0x83, 2048, 196_608);
        let entry = &mut sector[TABLE_OFFSET..TABLE_OFFSET + ENTRY_BYTES];
        entry[1..4].fill(0xFF);
        entry[5..8].fill(0xFF);
        let table = parse_mbr(&sector, DISK_SECTORS);
        assert_eq!(table.state, TableState::Valid);
        assert_eq!(
            table.entries[0],
            Some(Entry {
                boot: 0x80,
                type_id: 0x83,
                start_lba: 2048,
                sector_count: 196_608,
            })
        );
    }

    #[test]
    fn partition_ending_exactly_at_the_disk_end_is_valid() {
        let mut sector = signed_master_boot_record();
        stamp_entry(&mut sector, 0, 0x00, 0x83, 2048, (DISK_SECTORS - 2048) as u32);
        let table = parse_mbr(&sector, DISK_SECTORS);
        assert_eq!(table.state, TableState::Valid);
    }

    #[test]
    fn partition_reaching_past_the_disk_end_is_corrupt_and_still_reported() {
        let mut sector = signed_master_boot_record();
        stamp_entry(&mut sector, 0, 0x00, 0x83, 2048, (DISK_SECTORS - 2047) as u32);
        let table = parse_mbr(&sector, DISK_SECTORS);
        assert_eq!(table.state, TableState::Corrupt);
        assert!(table.entries[0].is_some());
    }

    #[test]
    fn entry_with_a_type_but_zero_sectors_is_corrupt() {
        let mut sector = signed_master_boot_record();
        stamp_entry(&mut sector, 0, 0x00, 0x83, 2048, 0);
        let table = parse_mbr(&sector, DISK_SECTORS);
        assert_eq!(table.state, TableState::Corrupt);
        assert!(table.entries[0].is_some());
    }

    #[test]
    fn entry_containing_the_table_itself_is_corrupt() {
        let mut sector = signed_master_boot_record();
        stamp_entry(&mut sector, 0, 0x00, 0x83, 0, 204_800);
        let table = parse_mbr(&sector, DISK_SECTORS);
        assert_eq!(table.state, TableState::Corrupt);
    }

    #[test]
    fn maximum_start_lba_on_a_small_disk_is_corrupt_without_wrapping() {
        let mut sector = signed_master_boot_record();
        stamp_entry(&mut sector, 0, 0x00, 0x83, u32::MAX, u32::MAX);
        let table = parse_mbr(&sector, DISK_SECTORS);
        assert_eq!(table.state, TableState::Corrupt);
    }

    #[test]
    fn maximum_extent_on_a_maximum_disk_is_valid_because_arithmetic_never_wraps() {
        let mut sector = signed_master_boot_record();
        stamp_entry(&mut sector, 0, 0x00, 0x83, u32::MAX, u32::MAX);
        let table = parse_mbr(&sector, u64::MAX);
        assert_eq!(table.state, TableState::Valid);
    }

    #[test]
    fn any_entry_beyond_a_tiny_disk_is_corrupt() {
        let mut sector = signed_master_boot_record();
        stamp_entry(&mut sector, 0, 0x00, 0x83, 2048, 16);
        let table = parse_mbr(&sector, 16);
        assert_eq!(table.state, TableState::Corrupt);
    }

    #[test]
    fn missing_signature_is_no_signature_with_no_slots_reported() {
        let mut sector = [0u8; SECTOR_BYTES];
        stamp_entry(&mut sector, 0, 0x80, 0x83, 2048, 196_608);
        let table = parse_mbr(&sector, DISK_SECTORS);
        assert_eq!(table.state, TableState::NoSignature);
        assert_eq!(table.entries, [None; ENTRY_COUNT]);
    }

    #[test]
    fn one_wrong_signature_byte_is_no_signature() {
        let mut sector = signed_master_boot_record();
        stamp_entry(&mut sector, 0, 0x80, 0x83, 2048, 196_608);
        sector[SIGNATURE_OFFSET + 1] = 0xAB;
        let table = parse_mbr(&sector, DISK_SECTORS);
        assert_eq!(table.state, TableState::NoSignature);
        assert_eq!(table.entries, [None; ENTRY_COUNT]);
    }

    #[test]
    fn all_zero_table_bytes_is_empty() {
        let sector = signed_master_boot_record();
        let table = parse_mbr(&sector, DISK_SECTORS);
        assert_eq!(table.state, TableState::Empty);
        assert_eq!(table.entries, [None; ENTRY_COUNT]);
    }

    #[test]
    fn one_zero_byte_in_the_signature_turns_the_same_table_to_no_signature() {
        let mut sector = [0u8; SECTOR_BYTES];
        sector[SIGNATURE_OFFSET] = 0x55;
        let table = parse_mbr(&sector, DISK_SECTORS);
        assert_eq!(table.state, TableState::NoSignature);
    }

    #[test]
    fn type_zero_slots_stay_empty_however_dirty_their_bytes() {
        let mut sector = signed_master_boot_record();
        let entry = &mut sector[TABLE_OFFSET + ENTRY_BYTES..][..ENTRY_BYTES];
        entry[0] = 0x80;
        entry[5..].fill(0xCD);
        let table = parse_mbr(&sector, DISK_SECTORS);
        assert_eq!(table.state, TableState::Valid);
        assert_eq!(table.entries, [None; ENTRY_COUNT]);
    }

    #[test]
    fn adjacent_partitions_do_not_overlap() {
        let mut sector = signed_master_boot_record();
        stamp_entry(&mut sector, 0, 0x00, 0x83, 2048, 100_352);
        stamp_entry(&mut sector, 1, 0x00, 0x82, 102_400, 102_400);
        let table = parse_mbr(&sector, DISK_SECTORS);
        assert_eq!(table.state, TableState::Valid);
    }

    #[test]
    fn one_shared_sector_between_partitions_is_corrupt() {
        let mut sector = signed_master_boot_record();
        stamp_entry(&mut sector, 0, 0x00, 0x83, 2048, 100_353);
        stamp_entry(&mut sector, 1, 0x00, 0x82, 102_400, 102_399);
        let table = parse_mbr(&sector, DISK_SECTORS);
        assert_eq!(table.state, TableState::Corrupt);
    }

    #[test]
    fn fully_contained_partition_is_corrupt() {
        let mut sector = signed_master_boot_record();
        stamp_entry(&mut sector, 0, 0x00, 0x83, 2048, 196_608);
        stamp_entry(&mut sector, 1, 0x00, 0x0b, 4096, 2_048);
        let table = parse_mbr(&sector, DISK_SECTORS);
        assert_eq!(table.state, TableState::Corrupt);
    }
}
