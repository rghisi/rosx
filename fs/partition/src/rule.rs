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

    #[test]
    fn harness_stamped_master_disk_shape_is_empty_exposing_the_whole_disk() {
        let mut sector = [0u8; SECTOR_BYTES];
        sector[SECTOR_BYTES - 2] = SIGNATURE[0];
        sector[SECTOR_BYTES - 1] = SIGNATURE[1];
        let one_mib_in_sectors = (1024 * 1024 / SECTOR_BYTES) as u64;
        let table = parse_mbr(&sector, one_mib_in_sectors);
        assert_eq!(table.state, TableState::Empty);
        assert_eq!(exposure(&table), Exposure::Whole { table: TableState::Empty });
    }

    #[test]
    fn ten_thousand_seeded_sectors_never_panic_and_valid_exposures_never_overlap() {
        let disk_sizes = [0u64, 1, 63, 2048, 204_800, u64::MAX];
        let mut partitions_seen = 0u32;
        let mut whole_seen = 0u32;
        for round in 0..10_000u64 {
            let sector = seeded_sector(round);
            for disk_sectors in disk_sizes {
                let table = parse_mbr(&sector, disk_sectors);
                match exposure(&table) {
                    Exposure::Whole { .. } => whole_seen += 1,
                    Exposure::Partitions(slices) => {
                        partitions_seen += 1;
                        assert!(!slices.is_empty());
                        assert_eq!(table.state, TableState::Valid);
                        assert!(slices.len() <= ENTRY_COUNT);
                        for slice in &slices {
                            let entry = table.entries[slice.slot as usize].unwrap();
                            assert!(is_supported(slice.type_id));
                            assert_eq!(
                                (slice.type_id, slice.start_lba, slice.sector_count),
                                (entry.type_id, entry.start_lba, entry.sector_count)
                            );
                        }
                        for (index, first) in slices.iter().enumerate() {
                            for second in &slices[index + 1..] {
                                let first_end =
                                    u64::from(first.start_lba) + u64::from(first.sector_count);
                                let second_end =
                                    u64::from(second.start_lba) + u64::from(second.sector_count);
                                assert!(
                                    u64::from(first.start_lba) >= second_end
                                        || u64::from(second.start_lba) >= first_end
                                );
                            }
                        }
                    }
                }
            }
        }
        assert!(partitions_seen > 0);
        assert!(whole_seen > 0);
    }

    fn seeded_sector(round: u64) -> [u8; SECTOR_BYTES] {
        let mut state = 0x9E37_79B9_7F4A_7C15 ^ (round.wrapping_mul(0xFF51_AF7D_2723_EE6B));
        let mut sector = [0u8; SECTOR_BYTES];
        for byte in sector.iter_mut() {
            *byte = next_random(&mut state) as u8;
        }
        if round % 2 == 1 {
            sector[SECTOR_BYTES - 2] = SIGNATURE[0];
            sector[SECTOR_BYTES - 1] = SIGNATURE[1];
        }
        sector
    }

    fn next_random(state: &mut u64) -> u64 {
        *state ^= *state << 13;
        *state ^= *state >> 7;
        *state ^= *state << 17;
        *state
    }
}
