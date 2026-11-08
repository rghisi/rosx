use rxpart::{exposure, parse_mbr, Entry, Exposure, Slice, TableState, SECTOR_BYTES};

const DISK_SECTORS: u64 = 204_800;
const ERA_DISK_SECTORS: u64 = 1_000_944;

fn entry(boot: u8, type_id: u8, start_lba: u32, sector_count: u32) -> Entry {
    Entry {
        boot,
        type_id,
        start_lba,
        sector_count,
    }
}

fn parsed(sector: &[u8; SECTOR_BYTES]) -> rxpart::Table {
    parse_mbr(sector, DISK_SECTORS)
}

fn partitions_of(sector: &[u8; SECTOR_BYTES]) -> Vec<Slice> {
    let table = parsed(sector);
    match exposure(&table) {
        Exposure::Partitions(slices) => {
            assert_eq!(table.state, TableState::Valid);
            slices
        }
        Exposure::Whole { table } => panic!("expected partitions, verdict was {table:?}"),
    }
}

fn partitions_of_disk(sector: &[u8; SECTOR_BYTES], disk_sectors: u64) -> Vec<Slice> {
    match exposure(&parse_mbr(sector, disk_sectors)) {
        Exposure::Partitions(slices) => slices,
        Exposure::Whole { table } => panic!("expected partitions, verdict was {table:?}"),
    }
}

fn whole_exposure_of(sector: &[u8; SECTOR_BYTES]) -> TableState {
    let table = parsed(sector);
    match exposure(&table) {
        Exposure::Whole { table } => table,
        Exposure::Partitions(slices) => panic!("expected whole-disk exposure, verdict was {slices:?}"),
    }
}

fn slot(sector: &[u8; SECTOR_BYTES], index: usize) -> [u8; 16] {
    sector[rxpart::TABLE_OFFSET + index * rxpart::ENTRY_BYTES..][..16].try_into().unwrap()
}

#[test]
fn sfdisk_legacy_63_layout_with_real_chs_bytes_exposes_both_partitions() {
    let sector: &[u8; SECTOR_BYTES] = include_bytes!("../fixtures/sfdisk-legacy-63-chs.mbr");
    let table = parsed(sector);
    assert_eq!(table.entries[0], Some(entry(0x80, 0x0c, 63, 50_000)));
    assert_eq!(table.entries[1], Some(entry(0x00, 0x82, 51_200, 40_000)));
    assert_eq!(table.entries[2..], [None, None]);
    assert_eq!(&sector[447..450], &[0x01, 0x01, 0x00]);
    assert_eq!(&sector[451..454], &[0x1d, 0x29, 0x03]);
    assert_eq!(
        partitions_of(sector),
        vec![
            Slice {
                slot: 0,
                type_id: 0x0c,
                start_lba: 63,
                sector_count: 50_000,
            },
            Slice {
                slot: 1,
                type_id: 0x82,
                start_lba: 51_200,
                sector_count: 40_000,
            },
        ]
    );
}

#[test]
fn sfdisk_aligned_pair_exposes_both_slices() {
    let sector: &[u8; SECTOR_BYTES] = include_bytes!("../fixtures/sfdisk-aligned-pair.mbr");
    let table = parsed(sector);
    assert_eq!(table.entries[0], Some(entry(0x00, 0x83, 2048, 81_920)));
    assert_eq!(table.entries[1], Some(entry(0x00, 0x83, 83_968, 81_920)));
    assert_eq!(
        partitions_of(sector),
        vec![
            Slice {
                slot: 0,
                type_id: 0x83,
                start_lba: 2048,
                sector_count: 81_920,
            },
            Slice {
                slot: 1,
                type_id: 0x83,
                start_lba: 83_968,
                sector_count: 81_920,
            },
        ]
    );
}

#[test]
fn sfdisk_four_primaries_expose_four_slices() {
    let sector: &[u8; SECTOR_BYTES] = include_bytes!("../fixtures/sfdisk-four-primaries.mbr");
    let table = parsed(sector);
    assert_eq!(table.entries[0], Some(entry(0x80, 0x0b, 2048, 50_000)));
    assert_eq!(table.entries[1], Some(entry(0x00, 0x83, 53_248, 50_000)));
    assert_eq!(table.entries[2], Some(entry(0x00, 0x82, 104_448, 50_000)));
    assert_eq!(table.entries[3], Some(entry(0x00, 0x16, 155_648, 30_000)));
    assert_eq!(partitions_of(sector).len(), 4);
}

#[test]
fn fdisk_deleted_slot_leaves_a_true_hole_at_index_one() {
    let sector: &[u8; SECTOR_BYTES] = include_bytes!("../fixtures/sfdisk-deleted-slot1.mbr");
    let table = parsed(sector);
    assert_eq!(table.entries[1], None);
    assert_eq!(slot(sector, 1), [0u8; 16]);
    let slices = partitions_of(sector);
    let slots: Vec<u8> = slices.iter().map(|slice| slice.slot).collect();
    assert_eq!(slots, vec![0, 2, 3]);
}

#[test]
fn sfdisk_dos_extended_container_is_recognized_and_refused() {
    let sector: &[u8; SECTOR_BYTES] = include_bytes!("../fixtures/sfdisk-container-05.mbr");
    let table = parsed(sector);
    assert_eq!(table.entries[0], Some(entry(0x00, 0x05, 2048, 150_000)));
    assert_eq!(whole_exposure_of(sector), TableState::Valid);
}

#[test]
fn parted_lba_extended_container_with_logicals_is_recognized_and_refused() {
    let sector: &[u8; SECTOR_BYTES] = include_bytes!("../fixtures/parted-extended-0f-logical.mbr");
    let table = parsed(sector);
    assert_eq!(table.entries[0], Some(entry(0x00, 0x0f, 1049, 203_751)));
    assert_eq!(whole_exposure_of(sector), TableState::Valid);
}

#[test]
fn sfdisk_data_beside_container_exposes_only_the_data_slice() {
    let sector: &[u8; SECTOR_BYTES] = include_bytes!("../fixtures/sfdisk-data-and-container.mbr");
    let table = parsed(sector);
    assert_eq!(table.entries[0], Some(entry(0x80, 0x83, 2048, 90_000)));
    assert_eq!(table.entries[1], Some(entry(0x00, 0x0f, 92_160, 100_000)));
    assert_eq!(
        partitions_of(sector),
        vec![Slice {
            slot: 0,
            type_id: 0x83,
            start_lba: 2048,
            sector_count: 90_000,
        }]
    );
}

#[test]
fn gdisk_protective_mbr_exposes_the_whole_disk_and_is_not_corrupt() {
    let sector: &[u8; SECTOR_BYTES] = include_bytes!("../fixtures/gdisk-protective-ee.mbr");
    let table = parsed(sector);
    assert_eq!(table.entries[0], Some(entry(0x00, 0xee, 1, 204_799)));
    assert_eq!(&sector[451..454], &[0xbe, 0x32, 0x0c]);
    assert_eq!(whole_exposure_of(sector), TableState::Valid);
}

#[test]
fn dos_fdisk_max_primary_keeps_the_era_geometry_and_no_disk_identifier() {
    let sector: &[u8; SECTOR_BYTES] = include_bytes!("../fixtures/dos-fdisk-max-primary.mbr");
    let table = parse_mbr(sector, ERA_DISK_SECTORS);
    assert_eq!(table.state, TableState::Valid);
    assert_eq!(table.entries[0], Some(entry(0x80, 0x06, 63, 998_865)));
    assert_eq!(table.entries[1..], [None, None, None]);
    assert_eq!(&sector[447..450], &[0x01, 0x01, 0x00]);
    assert_eq!(&sector[451..454], &[0x0f, 0xff, 0xde]);
    assert_eq!(&sector[440..444], &[0u8; 4]);
    assert_eq!(
        partitions_of_disk(sector, ERA_DISK_SECTORS),
        vec![Slice {
            slot: 0,
            type_id: 0x06,
            start_lba: 63,
            sector_count: 998_865,
        }]
    );
}

#[test]
fn dos_fdisk_primary_and_extended_container_expose_only_the_primary_slice() {
    let sector: &[u8; SECTOR_BYTES] = include_bytes!("../fixtures/dos-fdisk-primary-extended.mbr");
    let table = parse_mbr(sector, ERA_DISK_SECTORS);
    assert_eq!(table.state, TableState::Valid);
    assert_eq!(table.entries[0], Some(entry(0x80, 0x06, 63, 410_193)));
    assert_eq!(table.entries[1], Some(entry(0x00, 0x05, 410_256, 588_672)));
    assert_eq!(
        partitions_of_disk(sector, ERA_DISK_SECTORS),
        vec![Slice {
            slot: 0,
            type_id: 0x06,
            start_lba: 63,
            sector_count: 410_193,
        }]
    );
}
