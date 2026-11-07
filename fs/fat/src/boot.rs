use fs_core::error::FsError;

pub const SECTOR_BYTES: u16 = 512;
const BOOT_SECTOR_BYTES: usize = 512;
const MIN_SECTOR_BYTES: u16 = 256;
const MAX_SECTOR_BYTES: u16 = 4096;
const MAX_SECTORS_PER_CLUSTER: u32 = 128;
const ENTRY_BYTES: u64 = 32;
const FAT12_CLUSTER_LIMIT: u32 = 4085;
const FAT16_CLUSTER_LIMIT: u32 = 65525;

const OFFSET_BYTES_PER_SECTOR: usize = 0x0B;
const OFFSET_SECTORS_PER_CLUSTER: usize = 0x0D;
const OFFSET_RESERVED_SECTORS: usize = 0x0E;
const OFFSET_FAT_COUNT: usize = 0x10;
const OFFSET_ROOT_ENTRY_COUNT: usize = 0x11;
const OFFSET_TOTAL_SECTORS_16: usize = 0x13;
const OFFSET_FAT_SIZE_16: usize = 0x16;
const OFFSET_TOTAL_SECTORS_32: usize = 0x20;
const OFFSET_FAT_SIZE_32: usize = 0x24;
const OFFSET_ROOT_CLUSTER: usize = 0x2C;
const OFFSET_FSINFO_SECTOR: usize = 0x30;
const OFFSET_BOOT_SIGNATURE: usize = 0x1FE;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FatType {
    Fat12,
    Fat16,
    Fat32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RootLocation {
    Sectors { first: u64, entries: u32 },
    Cluster(u32),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BootSector {
    pub bytes_per_sector: u16,
    pub sectors_per_cluster: u32,
    pub fat_count: u32,
    pub reserved_sectors: u64,
    pub fat_sectors: u64,
    pub total_sectors: u64,
    pub first_data_sector: u64,
    pub cluster_count: u32,
    pub fat_type: FatType,
    pub root: RootLocation,
    pub fsinfo_sector: u32,
}

impl BootSector {
    pub fn parse(volume: &[u8], device_sectors: u64) -> Result<BootSector, FsError> {
        let volume = Self::check_present(volume)?;
        let bytes_per_sector = read16(volume, OFFSET_BYTES_PER_SECTOR);
        if bytes_per_sector.count_ones() != 1
            || !(MIN_SECTOR_BYTES..=MAX_SECTOR_BYTES).contains(&bytes_per_sector)
        {
            return Err(FsError::Corrupt);
        }
        if bytes_per_sector != SECTOR_BYTES {
            return Err(FsError::Unsupported);
        }
        let sectors_per_cluster = volume[OFFSET_SECTORS_PER_CLUSTER] as u32;
        if sectors_per_cluster.count_ones() != 1
            || sectors_per_cluster > MAX_SECTORS_PER_CLUSTER
        {
            return Err(FsError::Unsupported);
        }
        let reserved_sectors = read16(volume, OFFSET_RESERVED_SECTORS) as u64;
        if reserved_sectors == 0 {
            return Err(FsError::Corrupt);
        }
        let fat_count = volume[OFFSET_FAT_COUNT] as u32;
        if fat_count == 0 || fat_count > 2 {
            return Err(FsError::Corrupt);
        }
        let root_entry_count = read16(volume, OFFSET_ROOT_ENTRY_COUNT) as u32;
        let fat_size_16 = read16(volume, OFFSET_FAT_SIZE_16) as u64;
        let has_fat32_layout = fat_size_16 == 0 && root_entry_count == 0;
        let fat_sectors = if fat_size_16 == 0 {
            read32(volume, OFFSET_FAT_SIZE_32) as u64
        } else {
            fat_size_16
        };
        if fat_sectors == 0 {
            return Err(FsError::Corrupt);
        }
        let root_cluster = read32(volume, OFFSET_ROOT_CLUSTER);
        if has_fat32_layout {
            if root_cluster < 2 {
                return Err(FsError::Corrupt);
            }
        } else if root_entry_count == 0
            || u64::from(root_entry_count) % (u64::from(bytes_per_sector) / ENTRY_BYTES) != 0
        {
            return Err(FsError::Corrupt);
        }
        let total_sectors_16 = read16(volume, OFFSET_TOTAL_SECTORS_16) as u64;
        let total_sectors = if total_sectors_16 != 0 {
            total_sectors_16
        } else {
            read32(volume, OFFSET_TOTAL_SECTORS_32) as u64
        };
        if total_sectors == 0 {
            return Err(FsError::Corrupt);
        }
        let root_dir_sectors = if has_fat32_layout {
            0
        } else {
            (u64::from(root_entry_count) * ENTRY_BYTES + u64::from(bytes_per_sector) - 1)
                / u64::from(bytes_per_sector)
        };
        let root_start = reserved_sectors + u64::from(fat_count) * fat_sectors;
        let first_data_sector = root_start + root_dir_sectors;
        if total_sectors < first_data_sector {
            return Err(FsError::Corrupt);
        }
        if device_sectors < total_sectors {
            return Err(FsError::DeviceTooSmall);
        }
        let cluster_count = (total_sectors - first_data_sector) / u64::from(sectors_per_cluster);
        let cluster_count = u32::try_from(cluster_count).map_err(|_| FsError::VolumeTooLarge)?;
        if cluster_count == 0 {
            return Err(FsError::Corrupt);
        }
        let fat_type = classify(cluster_count);
        if (fat_type == FatType::Fat32) != has_fat32_layout {
            return Err(FsError::Corrupt);
        }
        let root = if has_fat32_layout {
            if root_cluster > cluster_count + 1 {
                return Err(FsError::Corrupt);
            }
            RootLocation::Cluster(root_cluster)
        } else {
            RootLocation::Sectors {
                first: root_start,
                entries: root_entry_count,
            }
        };
        let fsinfo_sector = if fat_type == FatType::Fat32 {
            let sector = read16(volume, OFFSET_FSINFO_SECTOR);
            if sector > 0 && u64::from(sector) < reserved_sectors {
                u32::from(sector)
            } else {
                0
            }
        } else {
            0
        };
        Ok(BootSector {
            bytes_per_sector,
            sectors_per_cluster,
            fat_count,
            reserved_sectors,
            fat_sectors,
            total_sectors,
            first_data_sector,
            cluster_count,
            fat_type,
            root,
            fsinfo_sector,
        })
    }

    fn check_present(volume: &[u8]) -> Result<&[u8], FsError> {
        if volume.len() < BOOT_SECTOR_BYTES {
            return Err(FsError::Corrupt);
        }
        let has_boot_signature = volume[OFFSET_BOOT_SIGNATURE] == 0x55
            && volume[OFFSET_BOOT_SIGNATURE + 1] == 0xAA;
        if has_boot_signature {
            Ok(&volume[..BOOT_SECTOR_BYTES])
        } else {
            Err(FsError::Unsupported)
        }
    }

    pub fn first_sector_of_cluster(&self, cluster: u32) -> Option<u64> {
        if cluster < 2 {
            return None;
        }
        let index = u64::from(cluster - 2);
        if index >= u64::from(self.cluster_count) {
            return None;
        }
        Some(self.first_data_sector + index * u64::from(self.sectors_per_cluster))
    }
}

fn classify(cluster_count: u32) -> FatType {
    if cluster_count < FAT12_CLUSTER_LIMIT {
        FatType::Fat12
    } else if cluster_count < FAT16_CLUSTER_LIMIT {
        FatType::Fat16
    } else {
        FatType::Fat32
    }
}

fn read16(volume: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([volume[offset], volume[offset + 1]])
}

fn read32(volume: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        volume[offset],
        volume[offset + 1],
        volume[offset + 2],
        volume[offset + 3],
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    const OFFSET_TYPE_STRING_LEGACY: usize = 0x36;
    const OFFSET_TYPE_STRING_32: usize = 0x5A;

    fn put16(bytes: &mut [u8], offset: usize, value: u16) {
        bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
    }

    fn put32(bytes: &mut [u8], offset: usize, value: u32) {
        bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }

    fn fat16_bytes() -> [u8; BOOT_SECTOR_BYTES] {
        let mut bytes = [0u8; BOOT_SECTOR_BYTES];
        bytes[0x00] = 0xEB;
        bytes[0x01] = 0x3C;
        bytes[0x02] = 0x90;
        put16(&mut bytes, OFFSET_BYTES_PER_SECTOR, 512);
        bytes[OFFSET_SECTORS_PER_CLUSTER] = 4;
        put16(&mut bytes, OFFSET_RESERVED_SECTORS, 1);
        bytes[OFFSET_FAT_COUNT] = 2;
        put16(&mut bytes, OFFSET_ROOT_ENTRY_COUNT, 512);
        bytes[0x15] = 0xF8;
        put16(&mut bytes, OFFSET_FAT_SIZE_16, 64);
        put32(&mut bytes, OFFSET_TOTAL_SECTORS_32, 65_536);
        bytes[OFFSET_TYPE_STRING_LEGACY..OFFSET_TYPE_STRING_LEGACY + 8].copy_from_slice(b"FAT16   ");
        bytes[OFFSET_BOOT_SIGNATURE] = 0x55;
        bytes[OFFSET_BOOT_SIGNATURE + 1] = 0xAA;
        bytes
    }

    fn fat32_bytes(total_sectors: u32) -> [u8; BOOT_SECTOR_BYTES] {
        let mut bytes = fat16_bytes();
        bytes[OFFSET_SECTORS_PER_CLUSTER] = 1;
        put16(&mut bytes, OFFSET_ROOT_ENTRY_COUNT, 0);
        put16(&mut bytes, OFFSET_FAT_SIZE_16, 0);
        put16(&mut bytes, OFFSET_RESERVED_SECTORS, 32);
        put32(&mut bytes, OFFSET_FAT_SIZE_32, 256);
        put32(&mut bytes, OFFSET_ROOT_CLUSTER, 2);
        put32(&mut bytes, OFFSET_TOTAL_SECTORS_32, total_sectors);
        bytes[OFFSET_TYPE_STRING_LEGACY..OFFSET_TYPE_STRING_LEGACY + 8].fill(0);
        bytes[OFFSET_TYPE_STRING_32..OFFSET_TYPE_STRING_32 + 8].copy_from_slice(b"FAT32   ");
        bytes
    }

    const FAT32_TOTAL_SECTORS: u32 = 32 + 512 + 65_525;

    #[test]
    fn parses_fat16_geometry() {
        let boot = BootSector::parse(&fat16_bytes(), 65_536).expect("valid fat16");
        assert_eq!(boot.bytes_per_sector, 512);
        assert_eq!(boot.sectors_per_cluster, 4);
        assert_eq!(boot.fat_count, 2);
        assert_eq!(boot.reserved_sectors, 1);
        assert_eq!(boot.fat_sectors, 64);
        assert_eq!(boot.total_sectors, 65_536);
        assert_eq!(boot.first_data_sector, 161);
        assert_eq!(boot.cluster_count, 16_343);
        assert_eq!(boot.fat_type, FatType::Fat16);
        assert_eq!(
            boot.root,
            RootLocation::Sectors {
                first: 129,
                entries: 512
            }
        );
    }

    #[test]
    fn parses_fat32_geometry_with_root_cluster_and_5a_signature() {
        let boot = BootSector::parse(&fat32_bytes(FAT32_TOTAL_SECTORS), 1_048_576).expect("valid fat32");
        assert_eq!(boot.reserved_sectors, 32);
        assert_eq!(boot.fat_sectors, 256);
        assert_eq!(boot.first_data_sector, 32 + 512);
        assert_eq!(boot.cluster_count, 65_525);
        assert_eq!(boot.fat_type, FatType::Fat32);
        assert_eq!(boot.root, RootLocation::Cluster(2));
    }

    #[test]
    fn detects_fat12_below_the_4085_cluster_limit() {
        let mut bytes = fat16_bytes();
        bytes[OFFSET_SECTORS_PER_CLUSTER] = 1;
        put16(&mut bytes, OFFSET_ROOT_ENTRY_COUNT, 224);
        put16(&mut bytes, OFFSET_FAT_SIZE_16, 16);
        put32(&mut bytes, OFFSET_TOTAL_SECTORS_32, 47 + 4_084);
        let boot = BootSector::parse(&bytes, 100_000).expect("fat12 geometry parses");
        assert_eq!(boot.fat_type, FatType::Fat12);
        assert_eq!(boot.cluster_count, 4_084);
    }

    #[test]
    fn detects_fat16_at_the_4085_cluster_limit() {
        let mut bytes = fat16_bytes();
        bytes[OFFSET_SECTORS_PER_CLUSTER] = 1;
        put16(&mut bytes, OFFSET_ROOT_ENTRY_COUNT, 224);
        put16(&mut bytes, OFFSET_FAT_SIZE_16, 16);
        put32(&mut bytes, OFFSET_TOTAL_SECTORS_32, 47 + 4_085);
        let boot = BootSector::parse(&bytes, 100_000).expect("fat16 geometry parses");
        assert_eq!(boot.fat_type, FatType::Fat16);
        assert_eq!(boot.cluster_count, 4_085);
    }

    #[test]
    fn fat32_layout_below_65525_clusters_is_corrupt() {
        let bytes = fat32_bytes(32 + 512 + 65_524);
        assert_eq!(BootSector::parse(&bytes, 1_048_576), Err(FsError::Corrupt));
    }

    #[test]
    fn fat16_layout_with_fat32_cluster_count_is_corrupt() {
        let mut bytes = fat16_bytes();
        bytes[OFFSET_SECTORS_PER_CLUSTER] = 1;
        put16(&mut bytes, OFFSET_FAT_SIZE_16, 256);
        put32(&mut bytes, OFFSET_TOTAL_SECTORS_32, 1 + 512 + 32 + 65_525);
        assert_eq!(BootSector::parse(&bytes, 2_097_152), Err(FsError::Corrupt));
    }

    #[test]
    fn rejects_missing_boot_signature_as_unsupported() {
        let mut bytes = fat16_bytes();
        bytes[OFFSET_BOOT_SIGNATURE] = 0x00;
        assert_eq!(
            BootSector::parse(&bytes, 65_536),
            Err(FsError::Unsupported)
        );
    }

    #[test]
    fn parses_volume_without_fat_type_string() {
        let mut bytes = fat16_bytes();
        bytes[OFFSET_TYPE_STRING_LEGACY..OFFSET_TYPE_STRING_LEGACY + 8].fill(0);
        assert!(BootSector::parse(&bytes, 65_536).is_ok());
    }

    #[test]
    fn rejects_truncated_input_as_corrupt() {
        assert_eq!(BootSector::parse(&fat16_bytes()[..256], 65_536), Err(FsError::Corrupt));
    }

    #[test]
    fn rejects_zero_and_misaligned_sector_sizes_as_corrupt() {
        for size in [0u16, 511, 1023] {
            let mut bytes = fat16_bytes();
            put16(&mut bytes, OFFSET_BYTES_PER_SECTOR, size);
            assert_eq!(
                BootSector::parse(&bytes, 65_536),
                Err(FsError::Corrupt),
                "sector size {size}"
            );
        }
    }

    #[test]
    fn rejects_sector_sizes_beyond_the_spec_as_corrupt() {
        let mut bytes = fat16_bytes();
        put16(&mut bytes, OFFSET_BYTES_PER_SECTOR, 8_192);
        assert_eq!(BootSector::parse(&bytes, 65_536), Err(FsError::Corrupt));
    }

    #[test]
    fn rejects_valid_non_512_sector_sizes_as_unsupported() {
        for size in [256u16, 1024, 4_096] {
            let mut bytes = fat16_bytes();
            put16(&mut bytes, OFFSET_BYTES_PER_SECTOR, size);
            assert_eq!(
                BootSector::parse(&bytes, 65_536),
                Err(FsError::Unsupported),
                "sector size {size}"
            );
        }
    }

    #[test]
    fn rejects_invalid_sectors_per_cluster_as_unsupported() {
        for spc in [0u8, 3, 96, 255] {
            let mut bytes = fat16_bytes();
            bytes[OFFSET_SECTORS_PER_CLUSTER] = spc;
            assert_eq!(
                BootSector::parse(&bytes, 65_536),
                Err(FsError::Unsupported),
                "sectors per cluster {spc}"
            );
        }
    }

    #[test]
    fn accepts_max_sectors_per_cluster() {
        let mut bytes = fat16_bytes();
        bytes[OFFSET_SECTORS_PER_CLUSTER] = 128;
        assert!(BootSector::parse(&bytes, 65_536).is_ok());
    }

    #[test]
    fn rejects_invalid_fat_counts() {
        for count in [0u8, 3] {
            let mut bytes = fat16_bytes();
            bytes[OFFSET_FAT_COUNT] = count;
            assert_eq!(
                BootSector::parse(&bytes, 65_536),
                Err(FsError::Corrupt),
                "fat count {count}"
            );
        }
    }

    #[test]
    fn rejects_zero_reserved_sectors() {
        let mut bytes = fat16_bytes();
        put16(&mut bytes, OFFSET_RESERVED_SECTORS, 0);
        assert_eq!(BootSector::parse(&bytes, 65_536), Err(FsError::Corrupt));
    }

    #[test]
    fn rejects_fat_without_any_fat_size() {
        let mut bytes = fat16_bytes();
        put16(&mut bytes, OFFSET_FAT_SIZE_16, 0);
        assert_eq!(BootSector::parse(&bytes, 65_536), Err(FsError::Corrupt));
    }

    #[test]
    fn rejects_fat16_layout_without_root_entries() {
        let mut bytes = fat16_bytes();
        put16(&mut bytes, OFFSET_ROOT_ENTRY_COUNT, 0);
        assert_eq!(BootSector::parse(&bytes, 65_536), Err(FsError::Corrupt));
    }

    #[test]
    fn rejects_root_entries_not_a_multiple_of_entries_per_sector() {
        let mut bytes = fat16_bytes();
        put16(&mut bytes, OFFSET_ROOT_ENTRY_COUNT, 223);
        assert_eq!(BootSector::parse(&bytes, 65_536), Err(FsError::Corrupt));
    }

    #[test]
    fn rejects_fat32_root_cluster_below_two() {
        for root_cluster in [0u32, 1] {
            let mut bytes = fat32_bytes(FAT32_TOTAL_SECTORS);
            put32(&mut bytes, OFFSET_ROOT_CLUSTER, root_cluster);
            assert_eq!(
                BootSector::parse(&bytes, 1_048_576),
                Err(FsError::Corrupt),
                "root cluster {root_cluster}"
            );
        }
    }

    #[test]
    fn rejects_fat32_root_cluster_beyond_the_data_area() {
        let mut bytes = fat32_bytes(FAT32_TOTAL_SECTORS);
        put32(&mut bytes, OFFSET_ROOT_CLUSTER, 65_527);
        assert_eq!(BootSector::parse(&bytes, 1_048_576), Err(FsError::Corrupt));
    }

    #[test]
    fn rejects_volume_without_total_sectors() {
        let mut bytes = fat16_bytes();
        put32(&mut bytes, OFFSET_TOTAL_SECTORS_32, 0);
        assert_eq!(BootSector::parse(&bytes, 65_536), Err(FsError::Corrupt));
    }

    #[test]
    fn prefers_total_sectors_16_when_present() {
        let mut bytes = fat16_bytes();
        put16(&mut bytes, OFFSET_TOTAL_SECTORS_16, 1_024);
        let boot = BootSector::parse(&bytes, 65_536).expect("total16 wins");
        assert_eq!(boot.total_sectors, 1_024);
        assert_eq!(boot.cluster_count, (1_024 - 161) / 4);
    }

    #[test]
    fn rejects_volume_smaller_than_its_own_headers() {
        let mut bytes = fat16_bytes();
        put32(&mut bytes, OFFSET_TOTAL_SECTORS_32, 100);
        assert_eq!(BootSector::parse(&bytes, 65_536), Err(FsError::Corrupt));
    }

    #[test]
    fn rejects_volume_without_a_single_cluster() {
        let mut bytes = fat16_bytes();
        put32(&mut bytes, OFFSET_TOTAL_SECTORS_32, 161);
        assert_eq!(BootSector::parse(&bytes, 65_536), Err(FsError::Corrupt));
    }

    #[test]
    fn rejects_device_smaller_than_the_volume() {
        assert_eq!(
            BootSector::parse(&fat16_bytes(), 1_000),
            Err(FsError::DeviceTooSmall)
        );
    }

    #[test]
    fn accepts_device_larger_than_the_volume() {
        let boot = BootSector::parse(&fat16_bytes(), 100_000).expect("trailing slack is fine");
        assert_eq!(boot.total_sectors, 65_536);
    }

    #[test]
    fn maps_clusters_to_sectors_with_reserved_numbers_rejected() {
        let boot = BootSector::parse(&fat16_bytes(), 65_536).expect("valid fat16");
        assert_eq!(boot.first_sector_of_cluster(0), None);
        assert_eq!(boot.first_sector_of_cluster(1), None);
        assert_eq!(boot.first_sector_of_cluster(2), Some(161));
        assert_eq!(boot.first_sector_of_cluster(3), Some(165));
        assert_eq!(boot.first_sector_of_cluster(16_344), Some(65_529));
        assert_eq!(boot.first_sector_of_cluster(16_345), None);
        assert_eq!(boot.first_sector_of_cluster(u32::MAX), None);
    }
}
