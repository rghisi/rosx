pub mod geometry;

use std::fs;
use std::io;
use std::path::Path;

use geometry::{FatKind, GeometryError, SECTOR_BYTES, VolumeGeometry};

const OEM_NAME: &[u8; 8] = b"MSDOS5.0";
const MEDIA_SUPERFLOPPY: u8 = 0xF0;
const OFFSET_BYTES_PER_SECTOR: usize = 0x0B;
const OFFSET_SECTORS_PER_CLUSTER: usize = 0x0D;
const OFFSET_RESERVED_SECTORS: usize = 0x0E;
const OFFSET_FAT_COUNT: usize = 0x10;
const OFFSET_ROOT_ENTRY_COUNT: usize = 0x11;
const OFFSET_TOTAL_SECTORS_16: usize = 0x13;
const OFFSET_MEDIA: usize = 0x15;
const OFFSET_FAT_SIZE_16: usize = 0x16;
const OFFSET_TOTAL_SECTORS_32: usize = 0x20;
const OFFSET_FAT_SIZE_32: usize = 0x24;
const OFFSET_ROOT_CLUSTER: usize = 0x2C;
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

pub struct FatDisk {
    geometry: VolumeGeometry,
    bytes: Vec<u8>,
}

impl FatDisk {
    pub fn new(kind: FatKind, capacity_bytes: u64) -> Result<FatDisk, GeometryError> {
        let geometry = geometry::plan_volume(kind, capacity_bytes)?;
        let mut disk = FatDisk {
            geometry,
            bytes: vec![0u8; (geometry.total_sectors * SECTOR_BYTES) as usize],
        };
        disk.write_boot_sector();
        disk.write_reserved_fat_entries();
        if kind == FatKind::Fat32 {
            disk.write_fsinfo();
            disk.write_fat32_backups();
        }
        Ok(disk)
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
        boot[OFFSET_MEDIA] = MEDIA_SUPERFLOPPY;
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
                boot[OFFSET_TYPE_STRING_32..OFFSET_TYPE_STRING_32 + 8]
                    .copy_from_slice(b"FAT32   ");
            }
        }
        boot[OFFSET_BOOT_SIGNATURE] = 0x55;
        boot[OFFSET_BOOT_SIGNATURE + 1] = 0xAA;
    }

    fn write_reserved_fat_entries(&mut self) {
        let fat_start = self.geometry.reserved_sectors as usize * SECTOR_BYTES as usize;
        let fat = &mut self.bytes[fat_start..fat_start + 2 * SECTOR_BYTES as usize];
        match self.geometry.kind {
            FatKind::Fat16 => {
                put16(fat, 0, 0xFFF);
                put16(fat, 2, 0xFFF);
            }
            FatKind::Fat32 => {
                put32(fat, 0, 0xFFFF_FFF0 | u32::from(MEDIA_SUPERFLOPPY & 0x0F));
                put32(fat, 4, 0x0FFF_FFFF);
                put32(fat, 8, 0x0FFF_FFFF);
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
        let backup =
            &disk.as_bytes()[6 * SECTOR_BYTES as usize..7 * SECTOR_BYTES as usize];
        assert_eq!(backup, &disk.as_bytes()[..SECTOR_BYTES as usize]);
    }

    #[test]
    fn fat_entries_open_with_media_and_end_markers() {
        let fat16 = FatDisk::new(FatKind::Fat16, 8 * MIB).unwrap();
        let fat = &fat16.as_bytes()[SECTOR_BYTES as usize..];
        assert_eq!(&fat[0..2], &0xFFFu16.to_le_bytes());
        assert_eq!(&fat[2..4], &0xFFFu16.to_le_bytes());

        let fat32 = FatDisk::new(FatKind::Fat32, 64 * MIB).unwrap();
        let fat = &fat32.as_bytes()[32 * SECTOR_BYTES as usize..];
        assert_eq!(&fat[0..4], &0xFFFF_FFF0u32.to_le_bytes());
        assert_eq!(&fat[4..8], &0x0FFF_FFFFu32.to_le_bytes());
        assert_eq!(&fat[8..12], &0x0FFF_FFFFu32.to_le_bytes());
    }
}
