use crate::mbr::{Slot, Table, TableState};
use alloc::vec::Vec;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Slice {
    pub slot: Slot,
    pub type_id: u8,
    pub start_lba: u32,
    pub sector_count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Exposure {
    Whole { table: TableState },
    Partitions(Vec<Slice>),
}

pub fn is_supported(type_id: u8) -> bool {
    type_id != 0 && !is_container(type_id)
}

pub fn exposure(table: &Table) -> Exposure {
    let slices = supported_slices(table);
    if slices.is_empty() {
        Exposure::Whole { table: table.state }
    } else {
        Exposure::Partitions(slices)
    }
}

fn is_container(type_id: u8) -> bool {
    matches!(type_id, CONTAINER_DOS_EXTENDED | CONTAINER_EXTENDED_LBA | CONTAINER_LINUX_EXTENDED | PROTECTIVE_GPT)
}

const CONTAINER_DOS_EXTENDED: u8 = 0x05;
const CONTAINER_EXTENDED_LBA: u8 = 0x0F;
const CONTAINER_LINUX_EXTENDED: u8 = 0x85;
const PROTECTIVE_GPT: u8 = 0xEE;

fn supported_slices(table: &Table) -> Vec<Slice> {
    if table.state != TableState::Valid {
        return Vec::new();
    }
    table
        .entries
        .iter()
        .enumerate()
        .filter_map(|(slot, entry)| match entry {
            Some(entry) if is_supported(entry.type_id) => Some(Slice {
                slot: slot as Slot,
                type_id: entry.type_id,
                start_lba: entry.start_lba,
                sector_count: entry.sector_count,
            }),
            _ => None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mbr::{ENTRY_BYTES, SIGNATURE, TABLE_OFFSET, parse_mbr, SECTOR_BYTES};
    use crate::mbr::ENTRY_COUNT;
    use alloc::vec;

    const DISK_SECTORS: u64 = 204_800;

    fn signed_master_boot_record() -> [u8; SECTOR_BYTES] {
        let mut sector = [0u8; SECTOR_BYTES];
        sector[SECTOR_BYTES - 2] = SIGNATURE[0];
        sector[SECTOR_BYTES - 1] = SIGNATURE[1];
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

    fn table_with(entries: &[(usize, u8, u32, u32)]) -> Table {
        let mut sector = signed_master_boot_record();
        for (slot, type_id, start_lba, sector_count) in entries {
            stamp_entry(&mut sector, *slot, 0x80, *type_id, *start_lba, *sector_count);
        }
        parse_mbr(&sector, DISK_SECTORS)
    }

    #[test]
    fn container_and_protective_types_are_recognized_but_unsupported() {
        assert!(!is_supported(CONTAINER_DOS_EXTENDED));
        assert!(!is_supported(CONTAINER_EXTENDED_LBA));
        assert!(!is_supported(CONTAINER_LINUX_EXTENDED));
        assert!(!is_supported(PROTECTIVE_GPT));
    }

    #[test]
    fn empty_and_unknown_types_split_on_support() {
        assert!(!is_supported(0x00));
        assert!(is_supported(0x01));
        assert!(is_supported(0x06));
        assert!(is_supported(0x83));
        assert!(is_supported(0xFE));
    }

    #[test]
    fn every_non_valid_state_exposes_the_whole_disk_with_its_state() {
        for state in [TableState::NoSignature, TableState::Empty, TableState::Corrupt] {
            let table = Table {
                state,
                entries: [None; ENTRY_COUNT],
            };
            assert_eq!(exposure(&table), Exposure::Whole { table: state });
        }
    }

    #[test]
    fn extended_only_disk_exposes_the_whole_disk_honestly() {
        let table = table_with(&[(0, CONTAINER_EXTENDED_LBA, 2048, DISK_SECTORS as u32 - 2048)]);
        assert_eq!(table.state, TableState::Valid);
        assert_eq!(exposure(&table), Exposure::Whole { table: TableState::Valid });
    }

    #[test]
    fn mixed_container_and_data_expose_only_the_data_slice() {
        let table = table_with(&[
            (0, CONTAINER_EXTENDED_LBA, 2048, 100_352),
            (2, 0x83, 104_448, 100_352),
        ]);
        assert_eq!(
            exposure(&table),
            Exposure::Partitions(vec![Slice {
                slot: 2,
                type_id: 0x83,
                start_lba: 104_448,
                sector_count: 100_352,
            }])
        );
    }

    #[test]
    fn protective_mbr_spanning_the_disk_is_valid_not_corrupt() {
        let table = table_with(&[(0, PROTECTIVE_GPT, 1, DISK_SECTORS as u32 - 1)]);
        assert_eq!(table.state, TableState::Valid);
        assert_eq!(exposure(&table), Exposure::Whole { table: TableState::Valid });
    }

    #[test]
    fn slices_carry_true_slot_indices_across_holes() {
        let table = table_with(&[(0, 0x0b, 2048, 102_400), (3, 0x83, 104_448, 100_352)]);
        assert_eq!(
            exposure(&table),
            Exposure::Partitions(vec![
                Slice {
                    slot: 0,
                    type_id: 0x0b,
                    start_lba: 2048,
                    sector_count: 102_400,
                },
                Slice {
                    slot: 3,
                    type_id: 0x83,
                    start_lba: 104_448,
                    sector_count: 100_352,
                },
            ])
        );
    }
}
