pub mod geometry;

use std::collections::BTreeSet;
use std::fmt;
use std::fs;
use std::io;
use std::path::Path;

pub use geometry::{FatKind, GeometryError, VolumeGeometry};
use geometry::SECTOR_BYTES;

const DIR_ENTRY_BYTES: usize = 32;
const DIR_ENTRIES_PER_SECTOR: u64 = SECTOR_BYTES / (DIR_ENTRY_BYTES as u64);
const SLOT_STEM: usize = 0;
const SLOT_EXTENSION: usize = 8;
const SLOT_ATTRIBUTES: usize = 0x0B;
const SLOT_CREATE_DATE: usize = 0x10;
const SLOT_ACCESS_DATE: usize = 0x12;
const SLOT_MODIFY_DATE: usize = 0x18;
const DOS_EPOCH_DATE: u16 = (1 << 5) | 1;
const SLOT_CLUSTER_HIGH: usize = 0x14;
const SLOT_CLUSTER_LOW: usize = 0x1A;
const SLOT_SIZE: usize = 0x1C;
const FAT16_CHAIN_END: u32 = 0xFFFF;
const FAT32_CHAIN_END: u32 = 0x0FFF_FFFF;

const OEM_NAME: &[u8; 8] = b"MSDOS5.0";
const MEDIA_FIXED_DISK: u8 = 0xF8;
const OFFSET_BYTES_PER_SECTOR: usize = 0x0B;
const OFFSET_SECTORS_PER_CLUSTER: usize = 0x0D;
const OFFSET_RESERVED_SECTORS: usize = 0x0E;
const OFFSET_FAT_COUNT: usize = 0x10;
const OFFSET_ROOT_ENTRY_COUNT: usize = 0x11;
const OFFSET_TOTAL_SECTORS_16: usize = 0x13;
const OFFSET_MEDIA: usize = 0x15;
const OFFSET_BIOS_SECTORS_PER_TRACK: usize = 0x18;
const OFFSET_BIOS_HEADS: usize = 0x1A;
const BIOS_SECTORS_PER_TRACK: u16 = 63;
const BIOS_HEADS: u16 = 16;
const OFFSET_FAT_SIZE_16: usize = 0x16;
const OFFSET_TOTAL_SECTORS_32: usize = 0x20;
const OFFSET_FAT_SIZE_32: usize = 0x24;
const OFFSET_ROOT_CLUSTER: usize = 0x2C;
const OFFSET_FSINFO_SECTOR: usize = 0x30;
const OFFSET_BACKUP_BOOT_SECTOR: usize = 0x32;
const OFFSET_TYPE_STRING_LEGACY: usize = 0x36;
const OFFSET_TYPE_STRING_32: usize = 0x5A;
const OFFSET_BOOT_SIGNATURE: usize = 0x1FE;
const FSINFO_LEAD_SIGNATURE: u32 = 0x4161_5252;
const FSINFO_FREE_COUNT: usize = 0x1E8;
const FSINFO_TRAIL_SIGNATURE: u32 = 0xAA55_0000;
const FSINFO_TRAIL_OFFSET: usize = 0x1FC;
const FSINFO_SECTOR: u64 = 1;
const BACKUP_BOOT_SECTOR: u64 = 6;
const BACKUP_FSINFO_SECTOR: u64 = 14;

#[derive(Debug, PartialEq, Eq)]
pub enum ImageError {
    Geometry(GeometryError),
    InvalidName(String),
    DuplicateName(String),
    FileTooLarge,
    Full,
}

impl fmt::Display for ImageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ImageError::Geometry(error) => write!(f, "{error}"),
            ImageError::InvalidName(name) => write!(f, "'{name}' does not fit the 8.3 name space"),
            ImageError::DuplicateName(name) => {
                write!(f, "'{name}' collides with an earlier entry")
            }
            ImageError::FileTooLarge => write!(f, "file does not fit the 32-bit size field"),
            ImageError::Full => write!(f, "volume out of clusters or directory entries"),
        }
    }
}

impl std::error::Error for ImageError {}

pub struct FatDisk {
    geometry: VolumeGeometry,
    bytes: Vec<u8>,
    names: BTreeSet<([u8; 8], [u8; 3])>,
    next_free_cluster: u32,
    root_slots_used: u32,
    root_last_cluster: u32,
    root_slots_in_last_cluster: u32,
}

impl FatDisk {
    pub fn new(kind: FatKind, capacity_bytes: u64) -> Result<FatDisk, ImageError> {
        let geometry = geometry::plan_volume(kind, capacity_bytes)?;
        let root_cluster = kind == FatKind::Fat32;
        let mut disk = FatDisk {
            geometry,
            bytes: vec![0u8; (geometry.total_sectors * SECTOR_BYTES) as usize],
            names: BTreeSet::new(),
            next_free_cluster: if root_cluster { 3 } else { 2 },
            root_slots_used: 0,
            root_last_cluster: if root_cluster { 2 } else { 0 },
            root_slots_in_last_cluster: 0,
        };
        disk.write_boot_sector();
        disk.write_reserved_fat_entries();
        if kind == FatKind::Fat32 {
            disk.write_fsinfo();
            disk.write_fat32_backups();
        }
        Ok(disk)
    }

    pub fn add_file(&mut self, name: &str, data: &[u8]) -> Result<(), ImageError> {
        let slot = slot_name(name)?;
        if !self.names.insert(slot) {
            return Err(ImageError::DuplicateName(String::from(name)));
        }
        if data.len() > u32::MAX as usize {
            return Err(ImageError::FileTooLarge);
        }
        if self.geometry.kind == FatKind::Fat16
            && self.root_slots_used as u64 >= self.geometry.root_entries
        {
            return Err(ImageError::Full);
        }
        let cluster_bytes = self.geometry.cluster_bytes() as usize;
        let data_clusters = (data.len() as u64).div_ceil(self.geometry.cluster_bytes());
        let root_extension = self.root_extension_needed();
        let available = self.geometry.cluster_count - (self.next_free_cluster - 2) as u64;
        if data_clusters + root_extension > available {
            return Err(ImageError::Full);
        }
        let first_cluster = self.next_free_cluster;
        for index in 0..data_clusters {
            let cluster = self.allocate_cluster();
            let next = if index + 1 == data_clusters {
                self.chain_end()
            } else {
                cluster + 1
            };
            self.write_fat_entry(cluster, next);
            let window = (index as usize * cluster_bytes)..(index as usize * cluster_bytes + cluster_bytes).min(data.len());
            if window.start < data.len() {
                let slice = &data[window];
                let disk_at = self.cluster_start_byte(cluster);
                self.bytes[disk_at..disk_at + slice.len()].copy_from_slice(slice);
            }
        }
        let first_cluster = if data_clusters == 0 { 0 } else { first_cluster };
        self.write_root_slot(slot, first_cluster, data.len() as u32);
        self.update_free_count();
        Ok(())
    }

    pub fn geometry(&self) -> &VolumeGeometry {
        &self.geometry
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn write_to(&self, path: &Path) -> io::Result<()> {
        fs::write(path, &self.bytes)
    }

    fn write_boot_sector(&mut self) {
        let geometry = self.geometry;
        let boot = &mut self.bytes[..SECTOR_BYTES as usize];
        boot[0] = 0xEB;
        boot[1] = 0x3C;
        boot[2] = 0x90;
        boot[3..11].copy_from_slice(OEM_NAME);
        put16(boot, OFFSET_BYTES_PER_SECTOR, SECTOR_BYTES as u16);
        boot[OFFSET_SECTORS_PER_CLUSTER] = geometry.sectors_per_cluster as u8;
        put16(
            boot,
            OFFSET_RESERVED_SECTORS,
            geometry.reserved_sectors as u16,
        );
        boot[OFFSET_FAT_COUNT] = 2;
        put16(boot, OFFSET_ROOT_ENTRY_COUNT, geometry.root_entries as u16);
        boot[OFFSET_MEDIA] = MEDIA_FIXED_DISK;
        put16(boot, OFFSET_BIOS_SECTORS_PER_TRACK, BIOS_SECTORS_PER_TRACK);
        put16(boot, OFFSET_BIOS_HEADS, BIOS_HEADS);
        match geometry.kind {
            FatKind::Fat16 => {
                put16(boot, OFFSET_FAT_SIZE_16, geometry.fat_sectors as u16);
                if geometry.total_sectors <= u64::from(u16::MAX) {
                    put16(
                        boot,
                        OFFSET_TOTAL_SECTORS_16,
                        geometry.total_sectors as u16,
                    );
                } else {
                    put32(boot, OFFSET_TOTAL_SECTORS_32, geometry.total_sectors as u32);
                }
                boot[OFFSET_TYPE_STRING_LEGACY..OFFSET_TYPE_STRING_LEGACY + 8]
                    .copy_from_slice(b"FAT16   ");
            }
            FatKind::Fat32 => {
                put32(boot, OFFSET_TOTAL_SECTORS_32, geometry.total_sectors as u32);
                put32(boot, OFFSET_FAT_SIZE_32, geometry.fat_sectors as u32);
                put32(boot, OFFSET_ROOT_CLUSTER, 2);
                put16(boot, OFFSET_FSINFO_SECTOR, FSINFO_SECTOR as u16);
                put16(boot, OFFSET_BACKUP_BOOT_SECTOR, BACKUP_BOOT_SECTOR as u16);
                boot[OFFSET_TYPE_STRING_32..OFFSET_TYPE_STRING_32 + 8]
                    .copy_from_slice(b"FAT32   ");
            }
        }
        boot[OFFSET_BOOT_SIGNATURE] = 0x55;
        boot[OFFSET_BOOT_SIGNATURE + 1] = 0xAA;
    }

    fn write_reserved_fat_entries(&mut self) {
        for start in self.fat_copy_starts() {
            let fat = &mut self.bytes[start..start + 2 * SECTOR_BYTES as usize];
            match self.geometry.kind {
                FatKind::Fat16 => {
                    put16(fat, 0, 0xFF00 | u16::from(MEDIA_FIXED_DISK));
                    put16(fat, 2, 0xFFFF);
                }
                FatKind::Fat32 => {
                    put32(fat, 0, 0xFFFF_FFF0 | u32::from(MEDIA_FIXED_DISK & 0x0F));
                    put32(fat, 4, 0x0FFF_FFFF);
                    put32(fat, 8, 0x0FFF_FFFF);
                }
            }
        }
    }

    fn write_fsinfo(&mut self) {
        let at = FSINFO_SECTOR as usize * SECTOR_BYTES as usize;
        let fsinfo = &mut self.bytes[at..at + SECTOR_BYTES as usize];
        fsinfo[FSINFO_FREE_COUNT..FSINFO_FREE_COUNT + 4]
            .copy_from_slice(&(self.geometry.cluster_count as u32).to_le_bytes());
        fsinfo[0x1E4..0x1E8].copy_from_slice(&FSINFO_LEAD_SIGNATURE.to_le_bytes());
        fsinfo[FSINFO_TRAIL_OFFSET..FSINFO_TRAIL_OFFSET + 4]
            .copy_from_slice(&FSINFO_TRAIL_SIGNATURE.to_le_bytes());
    }

    fn write_fat32_backups(&mut self) {
        let mut scratch = [0u8; SECTOR_BYTES as usize];
        scratch.copy_from_slice(&self.bytes[..SECTOR_BYTES as usize]);
        let backup_at = BACKUP_BOOT_SECTOR as usize * SECTOR_BYTES as usize;
        self.bytes[backup_at..backup_at + SECTOR_BYTES as usize].copy_from_slice(&scratch);
        let fsinfo_start = FSINFO_SECTOR as usize * SECTOR_BYTES as usize;
        scratch.copy_from_slice(&self.bytes[fsinfo_start..fsinfo_start + SECTOR_BYTES as usize]);
        let backup_at = BACKUP_FSINFO_SECTOR as usize * SECTOR_BYTES as usize;
        self.bytes[backup_at..backup_at + SECTOR_BYTES as usize].copy_from_slice(&scratch);
    }

    fn allocate_cluster(&mut self) -> u32 {
        let cluster = self.next_free_cluster;
        self.next_free_cluster += 1;
        cluster
    }

    fn chain_end(&self) -> u32 {
        match self.geometry.kind {
            FatKind::Fat16 => FAT16_CHAIN_END,
            FatKind::Fat32 => FAT32_CHAIN_END,
        }
    }

    fn cluster_start_byte(&self, cluster: u32) -> usize {
        self.geometry
            .first_sector_of_cluster(cluster)
            .expect("allocated clusters are inside the data area") as usize
            * SECTOR_BYTES as usize
    }

    fn fat_copy_starts(&self) -> [usize; 2] {
        [
            self.geometry.reserved_sectors as usize * SECTOR_BYTES as usize,
            (self.geometry.reserved_sectors + self.geometry.fat_sectors) as usize
                * SECTOR_BYTES as usize,
        ]
    }

    fn write_fat_entry(&mut self, cluster: u32, next: u32) {
        for start in self.fat_copy_starts() {
            match self.geometry.kind {
                FatKind::Fat16 => {
                    let at = start + u64::from(cluster) as usize * 2;
                    put16(&mut self.bytes[at..], 0, next as u16);
                }
                FatKind::Fat32 => {
                    let at = start + u64::from(cluster) as usize * 4;
                    put32(&mut self.bytes[at..], 0, next);
                }
            }
        }
    }

    fn root_extension_needed(&self) -> u64 {
        if self.geometry.kind != FatKind::Fat32 {
            return 0;
        }
        let slots_per_cluster =
            self.geometry.sectors_per_cluster as u64 * DIR_ENTRIES_PER_SECTOR;
        if self.root_slots_in_last_cluster as u64 == slots_per_cluster {
            1
        } else {
            0
        }
    }

    fn take_root_slot(&mut self) -> usize {
        match self.geometry.kind {
            FatKind::Fat16 => {
                let root_start = self.geometry.reserved_sectors + 2 * self.geometry.fat_sectors;
                (root_start * SECTOR_BYTES) as usize
                    + self.root_slots_used as usize * DIR_ENTRY_BYTES
            }
            FatKind::Fat32 => {
                let slots_per_cluster =
                    self.geometry.sectors_per_cluster * DIR_ENTRIES_PER_SECTOR as u32;
                if self.root_slots_in_last_cluster == slots_per_cluster {
                    let next = self.allocate_cluster();
                    self.write_fat_entry(self.root_last_cluster, next);
                    self.write_fat_entry(next, self.chain_end());
                    self.root_last_cluster = next;
                    self.root_slots_in_last_cluster = 0;
                }
                let slot = self.root_slots_in_last_cluster;
                self.root_slots_in_last_cluster += 1;
                self.cluster_start_byte(self.root_last_cluster)
                    + slot as usize * DIR_ENTRY_BYTES
            }
        }
    }

    fn write_root_slot(&mut self, slot: ([u8; 8], [u8; 3]), first_cluster: u32, size: u32) {
        let at = self.take_root_slot();
        let entry = &mut self.bytes[at..at + DIR_ENTRY_BYTES];
        entry[SLOT_STEM..SLOT_STEM + 8].copy_from_slice(&slot.0);
        entry[SLOT_EXTENSION..SLOT_EXTENSION + 3].copy_from_slice(&slot.1);
        entry[SLOT_ATTRIBUTES] = 0;
        put16(
            entry,
            SLOT_CLUSTER_HIGH,
            (first_cluster >> 16) as u16,
        );
        put16(entry, SLOT_CLUSTER_LOW, first_cluster as u16);
        put32(entry, SLOT_SIZE, size);
        put16(entry, SLOT_CREATE_DATE, DOS_EPOCH_DATE);
        put16(entry, SLOT_ACCESS_DATE, DOS_EPOCH_DATE);
        put16(entry, SLOT_MODIFY_DATE, DOS_EPOCH_DATE);
        self.root_slots_used += 1;
    }

    fn update_free_count(&mut self) {
        if self.geometry.kind != FatKind::Fat32 {
            return;
        }
        let free = (self.geometry.cluster_count - (self.next_free_cluster - 2) as u64) as u32;
        for sector in [FSINFO_SECTOR, BACKUP_FSINFO_SECTOR] {
            let at = sector as usize * SECTOR_BYTES as usize + FSINFO_FREE_COUNT;
            put32(&mut self.bytes[at..], 0, free);
        }
    }
}

fn slot_name(name: &str) -> Result<([u8; 8], [u8; 3]), ImageError> {
    let upper = name.to_ascii_uppercase();
    let (stem, extension) = match upper.rsplit_once('.') {
        Some((stem, extension)) => (stem, extension),
        None => (upper.as_str(), ""),
    };
    let fits = |field: &str, max: usize| {
        field.len() <= max
            && field
                .bytes()
                .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
    };
    if stem.is_empty() || !fits(stem, 8) || !fits(extension, 3) {
        return Err(ImageError::InvalidName(String::from(name)));
    }
    let mut stem_bytes = [b' '; 8];
    stem_bytes[..stem.len()].copy_from_slice(stem.as_bytes());
    let mut extension_bytes = [b' '; 3];
    extension_bytes[..extension.len()].copy_from_slice(extension.as_bytes());
    Ok((stem_bytes, extension_bytes))
}

impl From<GeometryError> for ImageError {
    fn from(error: GeometryError) -> Self {
        ImageError::Geometry(error)
    }
}

fn put16(bytes: &mut [u8], offset: usize, value: u16) {
    bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}

fn put32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;
    use rxfat::boot::{BootSector, FatType, RootLocation};

    const MIB: u64 = 1024 * 1024;

    #[test]
    fn fat16_boot_sector_round_trips_through_the_reader_parser() {
        let disk = FatDisk::new(FatKind::Fat16, 8 * MIB).unwrap();
        let geometry = disk.geometry();
        let boot = BootSector::parse(disk.as_bytes(), geometry.total_sectors).unwrap();
        assert_eq!(boot.bytes_per_sector, 512);
        assert_eq!(boot.sectors_per_cluster, geometry.sectors_per_cluster);
        assert_eq!(boot.fat_count, 2);
        assert_eq!(boot.reserved_sectors, geometry.reserved_sectors);
        assert_eq!(boot.fat_sectors, geometry.fat_sectors);
        assert_eq!(boot.total_sectors, geometry.total_sectors);
        assert_eq!(boot.fat_type, FatType::Fat16);
        assert_eq!(boot.cluster_count, geometry.cluster_count as u32);
        assert_eq!(
            boot.root,
            RootLocation::Sectors {
                first: geometry.reserved_sectors + 2 * geometry.fat_sectors,
                entries: geometry.root_entries as u32,
            }
        );
        assert_eq!(boot.first_sector_of_cluster(2), Some(geometry.first_data_sector));
    }

    #[test]
    fn fat32_boot_root_cluster_and_signatures_round_trip() {
        let disk = FatDisk::new(FatKind::Fat32, 64 * MIB).unwrap();
        let geometry = disk.geometry();
        let boot = BootSector::parse(disk.as_bytes(), geometry.total_sectors).unwrap();
        assert_eq!(boot.fat_type, FatType::Fat32);
        assert_eq!(boot.root, RootLocation::Cluster(2));
        assert_eq!(boot.cluster_count, geometry.cluster_count as u32);

        let fsinfo = &disk.as_bytes()
            [SECTOR_BYTES as usize + 0x1E4..SECTOR_BYTES as usize + 0x1E8];
        assert_eq!(fsinfo, &FSINFO_LEAD_SIGNATURE.to_le_bytes()[..]);
        assert_eq!(boot.fsinfo_sector, 1);
        let backup =
            &disk.as_bytes()[6 * SECTOR_BYTES as usize..7 * SECTOR_BYTES as usize];
        assert_eq!(backup, &disk.as_bytes()[..SECTOR_BYTES as usize]);
    }

    #[test]
    fn both_fat_copies_stay_byte_identical_through_allocation() {
        for kind in [FatKind::Fat16, FatKind::Fat32] {
            let capacity = match kind {
                FatKind::Fat16 => 8 * MIB,
                FatKind::Fat32 => 64 * MIB,
            };
            let mut disk = FatDisk::new(kind, capacity).unwrap();
            disk.add_file("ONE", &pattern(1_500)).unwrap();
            disk.add_file("TWO", &pattern(700)).unwrap();
            let geometry = disk.geometry();
            let fat_bytes = (geometry.fat_sectors * SECTOR_BYTES) as usize;
            let first = geometry.reserved_sectors as usize * SECTOR_BYTES as usize;
            let second = first + fat_bytes;
            assert_eq!(
                &disk.as_bytes()[first..first + fat_bytes],
                &disk.as_bytes()[second..second + fat_bytes],
                "{kind:?} copies must match"
            );
        }
    }

    #[test]
    fn fat_entries_open_with_media_and_end_markers() {
        let fat16 = FatDisk::new(FatKind::Fat16, 8 * MIB).unwrap();
        let fat = &fat16.as_bytes()[SECTOR_BYTES as usize..];
        assert_eq!(&fat[0..2], &0xFFF8u16.to_le_bytes());
        assert_eq!(&fat[2..4], &0xFFFFu16.to_le_bytes());

        let fat32 = FatDisk::new(FatKind::Fat32, 64 * MIB).unwrap();
        let fat = &fat32.as_bytes()[32 * SECTOR_BYTES as usize..];
        assert_eq!(&fat[0..4], &0xFFFF_FFF8u32.to_le_bytes());
        assert_eq!(&fat[4..8], &0x0FFF_FFFFu32.to_le_bytes());
        assert_eq!(&fat[8..12], &0x0FFF_FFFFu32.to_le_bytes());
    }

    struct ByteDevice<'a> {
        bytes: &'a [u8],
    }

    impl rxfat::device::BlockDevice for ByteDevice<'_> {
        fn block_count(&self) -> u64 {
            self.bytes.len() as u64 / SECTOR_BYTES
        }

        fn read_block(
            &mut self,
            block: u64,
            buf: &mut [u8],
        ) -> Result<(), fs_core::error::IoError> {
            let at = block as usize * SECTOR_BYTES as usize;
            let Some(region) = self.bytes.get(at..at + SECTOR_BYTES as usize) else {
                return Err(fs_core::error::IoError::Device);
            };
            if buf.len() != SECTOR_BYTES as usize {
                return Err(fs_core::error::IoError::Device);
            }
            buf.copy_from_slice(region);
            Ok(())
        }

        fn write_block(&mut self, block: u64, buf: &[u8]) -> Result<(), fs_core::error::IoError> {
            let _ = (block, buf);
            Err(fs_core::error::IoError::Device)
        }
    }

    fn open_reader(disk: &FatDisk) -> rxfat::FileSystem<ByteDevice<'_>> {
        rxfat::FileSystem::new(ByteDevice {
            bytes: disk.as_bytes(),
        })
        .expect("a fat-image volume must open in the reader")
    }

    fn pattern(length: usize) -> Vec<u8> {
        (0..length).map(|index| (index * 7 + 11) as u8).collect()
    }

    #[test]
    fn files_round_trip_byte_for_byte_through_the_engine() {
        let mut disk = FatDisk::new(FatKind::Fat16, 8 * MIB).unwrap();
        let big = pattern(5_000);
        let exact = pattern(512);
        disk.add_file("PI", &big).unwrap();
        disk.add_file("EXACT.TXT", &exact).unwrap();
        disk.add_file("EMPTY", &[]).unwrap();

        let mut volume = open_reader(&disk);
        let listing = volume.readdir(b"/").unwrap();
        let names: Vec<&[u8]> = listing.iter().map(|item| item.name.as_slice()).collect();
        assert_eq!(names, [b"pi".as_slice(), b"exact.txt".as_slice(), b"empty".as_slice()]);

        let handle = volume.open(b"/pi").unwrap();
        let mut whole = vec![0u8; big.len()];
        assert_eq!(volume.read(handle, 0, &mut whole).unwrap(), big.len());
        assert_eq!(whole, big);
        let mut straddling = [0u8; 64];
        assert_eq!(volume.read(handle, 480, &mut straddling).unwrap(), 64);
        assert_eq!(straddling, big[480..544]);

        let exact_handle = volume.open(b"/exact.txt").unwrap();
        let mut exact_read = vec![0u8; exact.len()];
        assert_eq!(
            volume.read(exact_handle, 0, &mut exact_read).unwrap(),
            exact.len()
        );
        assert_eq!(exact_read, exact);

        let empty_handle = volume.open(b"/empty").unwrap();
        assert_eq!(volume.read(empty_handle, 0, &mut straddling).unwrap(), 0);

        let geometry = disk.geometry();
        let root_start =
            (geometry.reserved_sectors + 2 * geometry.fat_sectors) as usize * SECTOR_BYTES as usize;
        let slot = &disk.as_bytes()[root_start..root_start + DIR_ENTRY_BYTES];
        assert_eq!(&slot[SLOT_CREATE_DATE..SLOT_CREATE_DATE + 2], &DOS_EPOCH_DATE.to_le_bytes());
        assert_eq!(&slot[SLOT_MODIFY_DATE..SLOT_MODIFY_DATE + 2], &DOS_EPOCH_DATE.to_le_bytes());
    }

    #[test]
    fn names_fold_to_lowercase_and_reject_outside_the_eight_three_space() {
        let mut disk = FatDisk::new(FatKind::Fat16, 8 * MIB).unwrap();
        disk.add_file("snake", &pattern(10)).unwrap();
        assert_eq!(
            disk.add_file("snake", &pattern(10)),
            Err(ImageError::DuplicateName(String::from("snake")))
        );
        assert!(matches!(
            disk.add_file("bad name", &pattern(1)),
            Err(ImageError::InvalidName(_))
        ));
        assert!(matches!(
            disk.add_file("123456789", &pattern(1)),
            Err(ImageError::InvalidName(_))
        ));
        assert!(matches!(
            disk.add_file(".hidden", &pattern(1)),
            Err(ImageError::InvalidName(_))
        ));
        let mut volume = open_reader(&disk);
        let listing = volume.readdir(b"/").unwrap();
        assert_eq!(listing[0].name, b"snake".to_vec());
    }

    #[test]
    fn fat32_root_grows_beyond_one_cluster_and_files_round_trip() {
        let mut disk = FatDisk::new(FatKind::Fat32, 64 * MIB).unwrap();
        let mut files = Vec::new();
        for index in 0..20u32 {
            let name = format!("FILE{index:03}");
            let data = pattern(600);
            disk.add_file(&name, &data).unwrap();
            files.push((name, data));
        }
        let mut volume = open_reader(&disk);
        let listing = volume.readdir(b"/").unwrap();
        assert_eq!(listing.len(), files.len());
        let handle = volume.open(b"/file017").unwrap();
        let mut read_back = vec![0u8; 600];
        assert_eq!(volume.read(handle, 0, &mut read_back).unwrap(), 600);
        assert_eq!(read_back, pattern(600));
    }

    #[test]
    fn a_full_volume_refuses_and_keeps_every_earlier_file() {
        let mut disk = FatDisk::new(FatKind::Fat16, 8 * MIB).unwrap();
        let block = pattern(8 * 1024);
        let mut accepted = 0usize;
        loop {
            let name = format!("B{accepted:07}");
            match disk.add_file(&name, &block) {
                Ok(()) => accepted += 1,
                Err(ImageError::Full) => break,
                Err(other) => panic!("unexpected error: {other}"),
            }
        }
        assert!(accepted > 100);
        let mut volume = open_reader(&disk);
        let listing = volume.readdir(b"/").unwrap();
        assert_eq!(listing.len(), accepted);
        let handle = volume.open(b"/b0000001").unwrap();
        let mut read_back = vec![0u8; block.len()];
        assert_eq!(volume.read(handle, 0, &mut read_back).unwrap(), block.len());
        assert_eq!(read_back, block);
    }
}
