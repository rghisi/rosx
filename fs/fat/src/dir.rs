use crate::boot::{BootSector, FatType, RootLocation};
use crate::device::BlockDevice;
use alloc::vec::Vec;
use fs_core::error::FsError;

pub const ENTRY_BYTES: usize = 32;
pub const ENTRIES_PER_SECTOR: usize = 512 / ENTRY_BYTES;

const ATTR_VOLUME_LABEL: u8 = 0x08;
const ATTR_LFN: u8 = 0x0F;
const ATTR_DIRECTORY: u8 = 0x10;

const OFFSET_ATTRS: usize = 0x0B;
const OFFSET_CLUSTER_HI: usize = 0x14;
const OFFSET_CLUSTER_LO: usize = 0x1A;
const OFFSET_SIZE: usize = 0x1C;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirEntry {
    pub name: Vec<u8>,
    pub is_directory: bool,
    pub first_cluster: u32,
    pub size: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DirLocation {
    Sectors { first: u64, entry_count: u32 },
    Chain(u32),
}

impl DirLocation {
    pub fn from_root(root: RootLocation) -> DirLocation {
        match root {
            RootLocation::Sectors { first, entries } => {
                DirLocation::Sectors {
                    first,
                    entry_count: entries,
                }
            }
            RootLocation::Cluster(cluster) => DirLocation::Chain(cluster),
        }
    }
}

pub fn name_matches(stored: &[u8], requested: &[u8]) -> bool {
    stored.eq_ignore_ascii_case(requested)
}

const SLOT_CREATE_DATE: usize = 0x10;
const SLOT_ACCESS_DATE: usize = 0x12;
const SLOT_MODIFY_DATE: usize = 0x18;
const DOS_EPOCH_DATE: u16 = (1 << 5) | 1;

pub fn fold_to_slot(name: &[u8]) -> Result<([u8; 8], [u8; 3]), FsError> {
    let (stem, extension) = match name.iter().rposition(|byte| *byte == b'.') {
        Some(dot) => (&name[..dot], &name[dot + 1..]),
        None => (name, b"".as_slice()),
    };
    let allowed = |byte: u8| matches!(byte, b'A'..=b'Z' | b'0'..=b'9' | b'_');
    let stem = stem.to_ascii_uppercase();
    let extension = extension.to_ascii_uppercase();
    if stem.is_empty()
        || stem.len() > 8
        || extension.len() > 3
        || !stem.iter().all(|byte| allowed(*byte))
        || !extension.iter().all(|byte| allowed(*byte))
    {
        return Err(FsError::InvalidName);
    }
    let mut stem_bytes = [b' '; 8];
    stem_bytes[..stem.len()].copy_from_slice(&stem);
    let mut extension_bytes = [b' '; 3];
    extension_bytes[..extension.len()].copy_from_slice(&extension);
    Ok((stem_bytes, extension_bytes))
}

pub fn make_slot_bytes(
    name: ([u8; 8], [u8; 3]),
    attrs: u8,
    first_cluster: u32,
    size: u32,
) -> [u8; ENTRY_BYTES] {
    let mut slot = [0u8; ENTRY_BYTES];
    slot[..8].copy_from_slice(&name.0);
    slot[8..11].copy_from_slice(&name.1);
    slot[OFFSET_ATTRS] = attrs;
    slot[OFFSET_CLUSTER_HI..OFFSET_CLUSTER_HI + 2]
        .copy_from_slice(&((first_cluster >> 16) as u16).to_le_bytes());
    slot[OFFSET_CLUSTER_LO..OFFSET_CLUSTER_LO + 2].copy_from_slice(&(first_cluster as u16).to_le_bytes());
    slot[OFFSET_SIZE..OFFSET_SIZE + 4].copy_from_slice(&size.to_le_bytes());
    for date_offset in [SLOT_CREATE_DATE, SLOT_ACCESS_DATE, SLOT_MODIFY_DATE] {
        slot[date_offset..date_offset + 2].copy_from_slice(&DOS_EPOCH_DATE.to_le_bytes());
    }
    slot
}

pub fn entry_sectors<D: BlockDevice>(
    device: &mut D,
    boot: &BootSector,
    location: DirLocation,
) -> Result<Vec<u64>, FsError> {
    match location {
        DirLocation::Sectors { first, entry_count } => {
            let sector_count = (u64::from(entry_count) * ENTRY_BYTES as u64).div_ceil(512);
            Ok((first..first + sector_count).collect())
        }
        DirLocation::Chain(start) => directory_sectors(device, boot, start),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlotSpot {
    Reuse(u64, usize),
    Append(u64, usize),
}

pub fn find_slot_spot<D: BlockDevice>(
    device: &mut D,
    sectors: &[u64],
    entry_limit: Option<u32>,
) -> Result<Option<SlotSpot>, FsError> {
    let limit = match entry_limit {
        Some(entries) => u64::from(entries),
        None => sectors.len() as u64 * ENTRIES_PER_SECTOR as u64,
    };
    let mut block = [0u8; 512];
    let mut deleted: Option<(u64, usize)> = None;
    let mut visited = 0u64;
    for sector in sectors {
        device.read_block(*sector, &mut block).map_err(FsError::Io)?;
        for slot_index in 0..ENTRIES_PER_SECTOR {
            if visited == limit {
                return Ok(deleted.map(|(s, o)| SlotSpot::Reuse(s, o)));
            }
            let offset = slot_index * ENTRY_BYTES;
            let first = block[offset];
            visited += 1;
            if first == 0x00 {
                return Ok(Some(match deleted {
                    Some((s, o)) => SlotSpot::Reuse(s, o),
                    None => SlotSpot::Append(*sector, offset),
                }));
            }
            if first == 0xE5 && deleted.is_none() {
                deleted = Some((*sector, offset));
            }
        }
    }
    Ok(deleted.map(|(s, o)| SlotSpot::Reuse(s, o)))
}

pub fn read_slot<D: BlockDevice>(
    device: &mut D,
    boot: &BootSector,
    location: DirLocation,
    name: &[u8],
) -> Result<(u64, usize, DirEntry), FsError> {
    let sectors = entry_sectors(device, boot, location)?;
    let limit = match location {
        DirLocation::Sectors { entry_count, .. } => u64::from(entry_count),
        DirLocation::Chain(_) => u64::MAX,
    };
    let mut block = [0u8; 512];
    let mut visited = 0u64;
    for sector in sectors {
        device.read_block(sector, &mut block).map_err(FsError::Io)?;
        for slot_index in 0..ENTRIES_PER_SECTOR {
            if visited == limit {
                return Err(FsError::NotFound);
            }
            let offset = slot_index * ENTRY_BYTES;
            if block[offset] == 0x00 {
                return Err(FsError::NotFound);
            }
            visited += 1;
            if block[offset] == 0xE5
                || block[offset + OFFSET_ATTRS] == ATTR_LFN
                || block[offset + OFFSET_ATTRS] & ATTR_VOLUME_LABEL != 0
            {
                continue;
            }
            if let Some(entry) = decode_entry(boot, &block[offset..offset + ENTRY_BYTES])?
                && name_matches(&entry.name, name)
            {
                return Ok((sector, offset, entry));
            }
        }
    }
    Err(FsError::NotFound)
}

pub fn write_slot_bytes<D: BlockDevice>(
    device: &mut D,
    sector: u64,
    offset: usize,
    slot: &[u8; ENTRY_BYTES],
) -> Result<(), FsError> {
    let mut block = [0u8; 512];
    device.read_block(sector, &mut block).map_err(FsError::Io)?;
    block[offset..offset + ENTRY_BYTES].copy_from_slice(slot);
    device.write_block(sector, &block).map_err(FsError::Io)
}

pub fn erase_slot<D: BlockDevice>(
    device: &mut D,
    sector: u64,
    offset: usize,
) -> Result<(), FsError> {
    let mut block = [0u8; 512];
    device.read_block(sector, &mut block).map_err(FsError::Io)?;
    block[offset] = 0xE5;
    device.write_block(sector, &block).map_err(FsError::Io)
}

pub fn read_directory<D: BlockDevice>(
    device: &mut D,
    boot: &BootSector,
    location: DirLocation,
) -> Result<Vec<DirEntry>, FsError> {
    let mut entries = Vec::new();
    let (sectors, limit) = match location {
        DirLocation::Sectors { first, entry_count } => {
            let sector_count = (u64::from(entry_count) * ENTRY_BYTES as u64).div_ceil(512);
            ((first..first + sector_count).collect(), u64::from(entry_count))
        }
        DirLocation::Chain(start) => (directory_sectors(device, boot, start)?, u64::MAX),
    };
    let mut visited = 0u64;
    let mut block = [0u8; 512];
    for sector in sectors {
        device.read_block(sector, &mut block).map_err(FsError::Io)?;
        for slot_index in 0..ENTRIES_PER_SECTOR {
            if visited == limit {
                return Ok(entries);
            }
            let slot = &block[slot_index * ENTRY_BYTES..slot_index * ENTRY_BYTES + ENTRY_BYTES];
            if slot[0] == 0x00 {
                return Ok(entries);
            }
            visited += 1;
            if slot[0] == 0xE5 || slot[OFFSET_ATTRS] == ATTR_LFN || slot[OFFSET_ATTRS] & ATTR_VOLUME_LABEL != 0 {
                continue;
            }
            if let Some(entry) = decode_entry(boot, slot)? {
                entries.push(entry);
            }
        }
    }
    Ok(entries)
}

fn directory_sectors<D: BlockDevice>(
    device: &mut D,
    boot: &BootSector,
    start_cluster: u32,
) -> Result<Vec<u64>, FsError> {
    let chain = crate::fat::cluster_chain(device, boot, start_cluster)?;
    let mut sectors = Vec::new();
    for cluster in chain {
        let first = boot
            .first_sector_of_cluster(cluster)
            .ok_or(FsError::Corrupt)?;
        sectors.extend(first..first + u64::from(boot.sectors_per_cluster));
    }
    Ok(sectors)
}

fn decode_entry(boot: &BootSector, slot: &[u8]) -> Result<Option<DirEntry>, FsError> {
    let name = folded_name(slot);
    if name.is_empty() || name == b".".as_slice() || name == b"..".as_slice() {
        return Ok(None);
    }
    let is_directory = slot[OFFSET_ATTRS] & ATTR_DIRECTORY != 0;
    let cluster_hi = u16::from_le_bytes([slot[OFFSET_CLUSTER_HI], slot[OFFSET_CLUSTER_HI + 1]]);
    let cluster_lo = u16::from_le_bytes([slot[OFFSET_CLUSTER_LO], slot[OFFSET_CLUSTER_LO + 1]]);
    if cluster_hi != 0 && boot.fat_type != FatType::Fat32 {
        return Err(FsError::Corrupt);
    }
    let first_cluster = u32::from(cluster_lo) | (u32::from(cluster_hi) << 16);
    let size = u32::from_le_bytes([
        slot[OFFSET_SIZE],
        slot[OFFSET_SIZE + 1],
        slot[OFFSET_SIZE + 2],
        slot[OFFSET_SIZE + 3],
    ]) as u64;
    if is_directory && size != 0 {
        return Err(FsError::Corrupt);
    }
    if (size > 0 || is_directory) && first_cluster < 2 {
        return Err(FsError::Corrupt);
    }
    if first_cluster >= 2 && boot.first_sector_of_cluster(first_cluster).is_none() {
        return Err(FsError::Corrupt);
    }
    Ok(Some(DirEntry {
        name,
        is_directory,
        first_cluster,
        size,
    }))
}

fn folded_name(slot: &[u8]) -> Vec<u8> {
    let mut name = Vec::with_capacity(12);
    for (index, byte) in slot[..8].iter().enumerate() {
        if *byte == 0x00 {
            break;
        }
        let byte = if index == 0 && *byte == 0x05 { 0xE5 } else { *byte };
        name.push(byte.to_ascii_lowercase());
    }
    while name.last() == Some(&b' ') {
        name.pop();
    }
    let extension = fold_extension(&slot[8..11]);
    if !extension.is_empty() {
        name.push(b'.');
        name.extend_from_slice(&extension);
    }
    name
}

fn fold_extension(extension: &[u8]) -> Vec<u8> {
    let mut folded = Vec::with_capacity(3);
    for byte in extension {
        if *byte == 0x00 {
            break;
        }
        folded.push(byte.to_ascii_lowercase());
    }
    while folded.last() == Some(&b' ') {
        folded.pop();
    }
    folded
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fat::fixtures::{
        fat16_boot, fat32_boot, fill_deleted, put_slot, slot_bytes, TestDevice,
    };
    use alloc::vec;

    fn file_entry(name: &[u8], is_directory: bool, first_cluster: u32, size: u64) -> DirEntry {
        DirEntry {
            name: name.to_vec(),
            is_directory,
            first_cluster,
            size,
        }
    }

    fn decoded(slot: &[u8]) -> Result<Option<DirEntry>, FsError> {
        decode_entry(&fat16_boot(), slot)
    }

    #[test]
    fn decodes_plain_file_entry() {
        let slot = slot_bytes(b"HELLO   ", b"TXT", 0x20, 9, 1_024);
        let entry = decoded(&slot).expect("decodes").expect("entry");
        assert_eq!(entry, file_entry(b"hello.txt", false, 9, 1_024));
    }

    #[test]
    fn folds_mixed_case_and_no_extension() {
        let slot = slot_bytes(b"MakeFiLE", b"   ", 0x20, 3, 7);
        let entry = decoded(&slot).expect("decodes").expect("entry");
        assert_eq!(entry, file_entry(b"makefile", false, 3, 7));
    }

    #[test]
    fn folds_full_base_with_longest_extension() {
        let slot = slot_bytes(b"ABCDEFGH", b"BAK", 0x20, 4, 8);
        let entry = decoded(&slot).expect("decodes").expect("entry");
        assert_eq!(entry, file_entry(b"abcdefgh.bak", false, 4, 8));
    }

    #[test]
    fn restores_deleted_marker_in_first_name_byte() {
        let mut slot = slot_bytes(b"\x05ILE    ", b"TXT", 0x20, 5, 1);
        slot[0] = 0x05;
        let entry = decoded(&slot).expect("decodes").expect("entry");
        assert_eq!(entry.name, vec![0xE5, b'i', b'l', b'e', b'.', b't', b'x', b't']);
    }

    #[test]
    fn skips_dot_and_dotdot_slots() {
        let dot = slot_bytes(b".       ", b"   ", ATTR_DIRECTORY, 2, 0);
        assert_eq!(decoded(&dot).expect("decodes"), None);
        let dotdot = slot_bytes(b"..      ", b"   ", ATTR_DIRECTORY, 0, 0);
        assert_eq!(decoded(&dotdot).expect("decodes"), None);
    }

    #[test]
    fn decodes_directory_entry() {
        let slot = slot_bytes(b"GAMES   ", b"   ", ATTR_DIRECTORY, 6, 0);
        let entry = decoded(&slot).expect("decodes").expect("entry");
        assert_eq!(entry, file_entry(b"games", true, 6, 0));
    }

    #[test]
    fn accepts_empty_file_pointing_at_cluster_zero() {
        let slot = slot_bytes(b"EMPTY   ", b"   ", 0x20, 0, 0);
        let entry = decoded(&slot).expect("decodes").expect("entry");
        assert_eq!(entry, file_entry(b"empty", false, 0, 0));
    }

    #[test]
    fn rejects_directory_with_nonzero_size() {
        let slot = slot_bytes(b"GAMES   ", b"   ", ATTR_DIRECTORY, 6, 512);
        assert_eq!(decoded(&slot), Err(FsError::Corrupt));
    }

    #[test]
    fn rejects_nonzero_cluster_hi_on_fat16() {
        let mut slot = slot_bytes(b"HELLO   ", b"TXT", 0x20, 9, 1);
        slot[OFFSET_CLUSTER_HI..OFFSET_CLUSTER_HI + 2].copy_from_slice(&1u16.to_le_bytes());
        assert_eq!(decoded(&slot), Err(FsError::Corrupt));
    }

    #[test]
    fn accepts_cluster_hi_on_fat32() {
        let mut slot = slot_bytes(b"HELLO   ", b"TXT", 0x20, 0, 0);
        slot[OFFSET_CLUSTER_HI..OFFSET_CLUSTER_HI + 2].copy_from_slice(&1u16.to_le_bytes());
        let boot = BootSector {
            cluster_count: 0x0001_0001,
            ..fat32_boot()
        };
        let entry = decode_entry(&boot, &slot).expect("decodes").expect("entry");
        assert_eq!(entry.first_cluster, 0x0001_0000);
    }

    #[test]
    fn rejects_cluster_outside_the_data_area() {
        let slot = slot_bytes(b"HELLO   ", b"TXT", 0x20, 5_000, 1);
        assert_eq!(decoded(&slot), Err(FsError::Corrupt));
    }

    #[test]
    fn rejects_directory_without_cluster() {
        let slot = slot_bytes(b"GAMES   ", b"   ", ATTR_DIRECTORY, 0, 0);
        assert_eq!(decoded(&slot), Err(FsError::Corrupt));
    }

    #[test]
    fn skips_deleted_labels_and_lfn_slots() {
        let boot = fat16_boot();
        let mut device = TestDevice::new(boot.total_sectors);
        let mut deleted = slot_bytes(b"GONE    ", b"TXT", 0x20, 2, 1);
        deleted[0] = 0xE5;
        let label = slot_bytes(b"MYLABEL ", b"   ", ATTR_VOLUME_LABEL, 0, 0);
        let lfn = slot_bytes(b"ABCDEFGH", b"IJK", ATTR_LFN, 0, 0);
        let live = slot_bytes(b"LIVE    ", b"EXE", 0x20, 2, 4);
        put_slot(&mut device, 17, 0, &deleted);
        put_slot(&mut device, 17, 1, &label);
        put_slot(&mut device, 17, 2, &lfn);
        put_slot(&mut device, 17, 3, &live);
        let entries = read_directory(&mut device, &boot, DirLocation::Sectors { first: 17, entry_count: 512 })
            .expect("reads");
        assert_eq!(entries, vec![file_entry(b"live.exe", false, 2, 4)]);
    }

    #[test]
    fn stops_at_the_first_empty_slot() {
        let boot = fat16_boot();
        let mut device = TestDevice::new(boot.total_sectors);
        let first = slot_bytes(b"ONE     ", b"   ", 0x20, 2, 1);
        let after = slot_bytes(b"TWO     ", b"   ", 0x20, 3, 1);
        put_slot(&mut device, 17, 0, &first);
        device.bytes[17usize * 512 + ENTRY_BYTES] = 0x00;
        put_slot(&mut device, 17, 2, &after);
        let entries = read_directory(&mut device, &boot, DirLocation::Sectors { first: 17, entry_count: 512 })
            .expect("reads");
        assert_eq!(entries, vec![file_entry(b"one", false, 2, 1)]);
    }

    #[test]
    fn reads_entries_across_root_sectors() {
        let boot = fat16_boot();
        let mut device = TestDevice::new(boot.total_sectors);
        fill_deleted(&mut device, 17);
        fill_deleted(&mut device, 18);
        for (slot_sector, slot_index, cluster) in [(17usize, 15usize, 2u16), (18usize, 0usize, 3), (18usize, 1usize, 4)] {
            let slot = slot_bytes(b"FILE    ", b"DAT", 0x20, cluster, 1);
            put_slot(&mut device, slot_sector as u64, slot_index, &slot);
        }
        let entries = read_directory(&mut device, &boot, DirLocation::Sectors { first: 17, entry_count: 512 })
            .expect("reads");
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].first_cluster, 2);
        assert_eq!(entries[2].first_cluster, 4);
    }

    #[test]
    fn reads_entries_across_a_directory_cluster_chain() {
        let boot = fat16_boot();
        let mut device = TestDevice::new(boot.total_sectors);
        let (fat_sector, fat_offset) = crate::fat::fixtures::fat16_entry_at(5);
        device.put16(fat_sector, fat_offset, 6);
        let (fat_sector, fat_offset) = crate::fat::fixtures::fat16_entry_at(6);
        device.put16(fat_sector, fat_offset, 0xFFF8);
        let first = slot_bytes(b"FIRST   ", b"   ", 0x20, 7, 1);
        let second = slot_bytes(b"SECOND  ", b"   ", 0x20, 8, 1);
        fill_deleted(&mut device, 52);
        fill_deleted(&mut device, 53);
        put_slot(&mut device, 52, 15, &first);
        put_slot(&mut device, 53, 0, &second);
        let entries = read_directory(&mut device, &boot, DirLocation::Chain(5)).expect("reads");
        assert_eq!(
            entries,
            vec![
                file_entry(b"first", false, 7, 1),
                file_entry(b"second", false, 8, 1),
            ]
        );
    }

    #[test]
    fn maps_root_locations_to_directories() {
        assert_eq!(
            DirLocation::from_root(RootLocation::Sectors { first: 17, entries: 512 }),
            DirLocation::Sectors { first: 17, entry_count: 512 }
        );
        assert_eq!(DirLocation::from_root(RootLocation::Cluster(2)), DirLocation::Chain(2));
    }

    #[test]
    fn matches_names_case_insensitively() {
        assert!(name_matches(b"snake", b"SNAKE"));
        assert!(name_matches(b"readme.txt", b"README.TXT"));
        assert!(!name_matches(b"snake", b"snak"));
        assert!(!name_matches(b"snake", b"snakee"));
    }

    #[test]
    fn fixed_root_region_without_terminator_still_yields_all_entries() {
        let boot = fat16_boot();
        let mut device = TestDevice::new(boot.total_sectors);
        for slot_index in 0..ENTRIES_PER_SECTOR {
            let slot = slot_bytes(b"FULL    ", b"BIN", 0x20, 2, 1);
            put_slot(&mut device, 17, slot_index, &slot);
        }
        let location = DirLocation::Sectors {
            first: 17,
            entry_count: ENTRIES_PER_SECTOR as u32,
        };
        let entries = read_directory(&mut device, &boot, location).expect("reads");
        assert_eq!(entries.len(), ENTRIES_PER_SECTOR);
    }

    #[test]
    fn folds_names_into_the_eight_three_space() {
        assert_eq!(
            fold_to_slot(b"snake").expect("stem only"),
            (*b"SNAKE   ", *b"   ")
        );
        assert_eq!(
            fold_to_slot(b"notes.txt").expect("stem and extension"),
            (*b"NOTES   ", *b"TXT")
        );
        assert_eq!(
            fold_to_slot(b"abcdefgh.bak").expect("longest legal"),
            (*b"ABCDEFGH", *b"BAK")
        );
    }

    #[test]
    fn rejects_names_outside_the_eight_three_space() {
        for name in [
            b"".as_slice(),
            b".hidden".as_slice(),
            b"ABCDEFGHI".as_slice(),
            b"notes.TXTX".as_slice(),
            b"bad name".as_slice(),
            b"a.b.txt".as_slice(),
        ] {
            assert_eq!(fold_to_slot(name), Err(FsError::InvalidName), "{name:?}");
        }
    }

    #[test]
    fn built_slots_decode_back_through_the_reader() {
        let boot = fat16_boot();
        let slot = make_slot_bytes(fold_to_slot(b"notes.txt").expect("fold"), 0x20, 9, 1_024);
        let entry = decode_entry(&boot, &slot).expect("decodes").expect("entry");
        assert_eq!(&entry.name, b"notes.txt");
        assert_eq!((entry.first_cluster, entry.size), (9, 1_024));
        assert!(!entry.is_directory);
    }

    #[test]
    fn slot_spot_prefers_the_first_deleted_slot_over_the_terminator() {
        let boot = fat16_boot();
        let mut device = TestDevice::new(boot.total_sectors);
        let live = slot_bytes(b"LIVE    ", b"   ", 0x20, 2, 1);
        let mut deleted = slot_bytes(b"GONE    ", b"   ", 0x20, 3, 1);
        deleted[0] = 0xE5;
        put_slot(&mut device, 17, 0, &live);
        put_slot(&mut device, 17, 1, &deleted);
        let spot = find_slot_spot(&mut device, &[17], Some(512))
            .expect("scan")
            .expect("spot");
        assert_eq!(spot, SlotSpot::Reuse(17, 32));

        let mut all_live = TestDevice::new(boot.total_sectors);
        for slot_index in 0..ENTRIES_PER_SECTOR {
            let slot = slot_bytes(b"FULL    ", b"   ", 0x20, 2, 1);
            put_slot(&mut all_live, 17, slot_index, &slot);
        }
        assert_eq!(
            find_slot_spot(&mut all_live, &[17], Some(ENTRIES_PER_SECTOR as u32))
                .expect("scan"),
            None
        );
    }

    #[test]
    fn writes_a_slot_and_finds_it_by_position() {
        let boot = fat16_boot();
        let mut device = TestDevice::new(boot.total_sectors);
        let slot = make_slot_bytes(fold_to_slot(b"new").expect("fold"), 0x20, 7, 42);
        write_slot_bytes(&mut device, 17, 0, &slot).expect("install");
        let (sector, offset, entry) =
            read_slot(&mut device, &boot, DirLocation::Sectors { first: 17, entry_count: 512 }, b"new")
                .expect("found");
        assert_eq!((sector, offset), (17, 0));
        assert_eq!((entry.first_cluster, entry.size), (7, 42));

        erase_slot(&mut device, sector, offset).expect("erase");
        assert_eq!(
            read_slot(
                &mut device,
                &boot,
                DirLocation::Sectors { first: 17, entry_count: 512 },
                b"new"
            ),
            Err(FsError::NotFound)
        );
        let entries = read_directory(
            &mut device,
            &boot,
            DirLocation::Sectors { first: 17, entry_count: 512 },
        )
        .expect("listing");
        assert!(entries.is_empty());
    }
}
