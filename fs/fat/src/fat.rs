use crate::boot::{BootSector, FatType, SECTOR_BYTES};
use crate::device::BlockDevice;
use alloc::vec::Vec;
use fs_core::error::FsError;

const FAT16_RESERVED_LOW: u32 = 0x0002;
const FAT16_RESERVED_HIGH: u32 = 0xFFF1;
const FAT16_END: u32 = 0xFFF8;
const FAT32_VALUE_MASK: u32 = 0x0FFF_FFFF;
const FAT32_RESERVED_HIGH: u32 = 0x0FFF_FFF1;
const FAT32_END: u32 = 0x0FFF_FFF8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClusterNext {
    End,
    Next(u32),
}

pub fn next_cluster<D: BlockDevice>(
    device: &mut D,
    boot: &BootSector,
    cluster: u32,
) -> Result<ClusterNext, FsError> {
    if boot.first_sector_of_cluster(cluster).is_none() {
        return Err(FsError::Corrupt);
    }
    let width = match boot.fat_type {
        FatType::Fat12 => return Err(FsError::Unsupported),
        FatType::Fat16 => 2u64,
        FatType::Fat32 => 4u64,
    };
    let entry_bytes = u64::from(cluster) * width;
    let sector =
        boot.reserved_sectors + entry_bytes / u64::from(boot.bytes_per_sector);
    let offset = (entry_bytes % u64::from(boot.bytes_per_sector)) as usize;
    let mut block = [0u8; SECTOR_BYTES as usize];
    device.read_block(sector, &mut block).map_err(FsError::Io)?;
    let value = if boot.fat_type == FatType::Fat16 {
        u32::from(block[offset]) | (u32::from(block[offset + 1]) << 8)
    } else {
        (u32::from(block[offset])
            | (u32::from(block[offset + 1]) << 8)
            | (u32::from(block[offset + 2]) << 16)
            | (u32::from(block[offset + 3]) << 24))
            & FAT32_VALUE_MASK
    };
    classify_entry(boot.fat_type, value)
}

pub fn cluster_chain<D: BlockDevice>(
    device: &mut D,
    boot: &BootSector,
    start_cluster: u32,
) -> Result<Vec<u32>, FsError> {
    let mut chain: Vec<u32> = Vec::new();
    let mut current = start_cluster;
    loop {
        chain.push(current);
        if chain.len() > boot.cluster_count as usize {
            return Err(FsError::Corrupt);
        }
        match next_cluster(device, boot, current)? {
            ClusterNext::End => return Ok(chain),
            ClusterNext::Next(next) => current = next,
        }
    }
}

fn classify_entry(fat_type: FatType, value: u32) -> Result<ClusterNext, FsError> {
    let (reserved_low, reserved_high, end_at) = match fat_type {
        FatType::Fat12 => return Err(FsError::Unsupported),
        FatType::Fat16 => (FAT16_RESERVED_LOW, FAT16_RESERVED_HIGH, FAT16_END),
        FatType::Fat32 => (FAT16_RESERVED_LOW, FAT32_RESERVED_HIGH, FAT32_END),
    };
    if value < reserved_low || (reserved_high..end_at).contains(&value) {
        return Err(FsError::Corrupt);
    }
    if value >= end_at {
        return Ok(ClusterNext::End);
    }
    Ok(ClusterNext::Next(value))
}

#[cfg(test)]
pub(crate) mod fixtures {
    use super::*;
    use crate::boot::RootLocation;
    use alloc::vec;
    use fs_core::error::IoError;

    pub struct TestDevice {
        pub bytes: Vec<u8>,
    }

    impl TestDevice {
        pub fn new(sectors: u64) -> TestDevice {
            TestDevice {
                bytes: vec![0u8; sectors as usize * SECTOR_BYTES as usize],
            }
        }

        pub fn put16(&mut self, sector: u64, offset: usize, value: u16) {
            let at = sector as usize * SECTOR_BYTES as usize + offset;
            self.bytes[at..at + 2].copy_from_slice(&value.to_le_bytes());
        }

        pub fn put32(&mut self, sector: u64, offset: usize, value: u32) {
            let at = sector as usize * SECTOR_BYTES as usize + offset;
            self.bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
        }
    }

    impl BlockDevice for TestDevice {
        fn block_count(&self) -> u64 {
            self.bytes.len() as u64 / SECTOR_BYTES as u64
        }

        fn read_block(&mut self, block: u64, buf: &mut [u8]) -> Result<(), IoError> {
            let at = block as usize * SECTOR_BYTES as usize;
            let Some(region) = self.bytes.get(at..at + SECTOR_BYTES as usize) else {
                return Err(IoError::Device);
            };
            if buf.len() != SECTOR_BYTES as usize {
                return Err(IoError::Device);
            }
            buf.copy_from_slice(region);
            Ok(())
        }
    }

    pub fn fat16_boot() -> BootSector {
        BootSector {
            bytes_per_sector: SECTOR_BYTES,
            sectors_per_cluster: 1,
            fat_count: 2,
            reserved_sectors: 1,
            fat_sectors: 8,
            total_sectors: 49 + 4_086,
            first_data_sector: 49,
            cluster_count: 4_086,
            fat_type: FatType::Fat16,
            root: RootLocation::Sectors {
                first: 17,
                entries: 512,
            },
        }
    }

    pub fn fat32_boot() -> BootSector {
        BootSector {
            fat_type: FatType::Fat32,
            cluster_count: 100,
            ..fat16_boot()
        }
    }

    pub fn fat16_entry_at(cluster: u32) -> (u64, usize) {
        let entry_bytes = u64::from(cluster) * 2;
        (1 + entry_bytes / 512, (entry_bytes % 512) as usize)
    }

    pub fn fat32_entry_at(cluster: u32) -> (u64, usize) {
        let entry_bytes = u64::from(cluster) * 4;
        (1 + entry_bytes / 512, (entry_bytes % 512) as usize)
    }
}

#[cfg(test)]
mod tests {
    use super::fixtures::*;
    use super::*;
    use alloc::vec;
    use fs_core::error::IoError;

    #[test]
    fn follows_a_fat16_chain() {
        let boot = fat16_boot();
        let mut device = TestDevice::new(boot.total_sectors);
        let (sector, offset) = fat16_entry_at(2);
        device.put16(sector, offset, 3);
        let (sector, offset) = fat16_entry_at(3);
        device.put16(sector, offset, 4);
        let (sector, offset) = fat16_entry_at(4);
        device.put16(sector, offset, 0xFFF8);
        assert_eq!(cluster_chain(&mut device, &boot, 2).expect("chain"), vec![2, 3, 4]);
    }

    #[test]
    fn ends_a_single_cluster_chain() {
        let boot = fat16_boot();
        let mut device = TestDevice::new(boot.total_sectors);
        let (sector, offset) = fat16_entry_at(7);
        device.put16(sector, offset, 0xFFFF);
        assert_eq!(cluster_chain(&mut device, &boot, 7).expect("chain"), vec![7]);
    }

    #[test]
    fn reads_only_the_first_fat() {
        let boot = fat16_boot();
        let mut device = TestDevice::new(boot.total_sectors);
        let (sector, offset) = fat16_entry_at(2);
        device.put16(sector, offset, 0xFFF8);
        assert_eq!(next_cluster(&mut device, &boot, 2).expect("first fat"), ClusterNext::End);
    }

    #[test]
    fn crosses_the_fat_sector_boundary_in_fat16() {
        let boot = fat16_boot();
        let mut device = TestDevice::new(boot.total_sectors);
        let last_entry_cluster = 256;
        let (sector, offset) = fat16_entry_at(last_entry_cluster);
        assert_eq!((sector, offset), (2, 0));
        device.put16(sector, offset, 0xFFF8);
        assert_eq!(
            next_cluster(&mut device, &boot, last_entry_cluster).expect("entry in fat sector 2"),
            ClusterNext::End
        );
    }

    #[test]
    fn follows_a_fat32_chain_and_ignores_reserved_high_bits() {
        let boot = fat32_boot();
        let mut device = TestDevice::new(boot.first_data_sector + boot.cluster_count as u64);
        let (sector, offset) = fat32_entry_at(3);
        device.put32(sector, offset, 0xF000_0004);
        let (sector, offset) = fat32_entry_at(4);
        device.put32(sector, offset, 0xFFFF_FFFF);
        assert_eq!(cluster_chain(&mut device, &boot, 3).expect("chain"), vec![3, 4]);
    }

    #[test]
    fn rejects_free_and_bad_fat16_entries() {
        for value in [0x0000u16, 0xFFF1, 0xFFF7] {
            let boot = fat16_boot();
            let mut device = TestDevice::new(boot.total_sectors);
            let (sector, offset) = fat16_entry_at(2);
            device.put16(sector, offset, value);
            assert_eq!(
                next_cluster(&mut device, &boot, 2),
                Err(FsError::Corrupt),
                "entry value {value:#06x}"
            );
        }
    }

    #[test]
    fn rejects_free_bad_and_reserved_one_fat32_entries() {
        for value in [0x0000_0000u32, 0x0000_0001, 0x0FFF_FFF7, 0x0FFF_FFF1] {
            let boot = fat32_boot();
            let mut device = TestDevice::new(boot.first_data_sector + boot.cluster_count as u64);
            let (sector, offset) = fat32_entry_at(3);
            device.put32(sector, offset, value);
            assert_eq!(
                next_cluster(&mut device, &boot, 3),
                Err(FsError::Corrupt),
                "entry value {value:#010x}"
            );
        }
    }

    #[test]
    fn rejects_a_chain_loop_beyond_the_cluster_count() {
        let boot = fat16_boot();
        let mut device = TestDevice::new(boot.total_sectors);
        let (sector, offset) = fat16_entry_at(2);
        device.put16(sector, offset, 3);
        let (sector, offset) = fat16_entry_at(3);
        device.put16(sector, offset, 2);
        assert_eq!(cluster_chain(&mut device, &boot, 2), Err(FsError::Corrupt));
    }

    #[test]
    fn rejects_a_chain_jump_outside_the_data_area() {
        let boot = fat16_boot();
        let mut device = TestDevice::new(boot.total_sectors);
        let outside = 2 + boot.cluster_count;
        let (sector, offset) = fat16_entry_at(2);
        device.put16(sector, offset, outside.try_into().unwrap());
        assert_eq!(cluster_chain(&mut device, &boot, 2), Err(FsError::Corrupt));
    }

    #[test]
    fn rejects_reserved_cluster_numbers_as_start() {
        let boot = fat16_boot();
        let mut device = TestDevice::new(boot.total_sectors);
        assert_eq!(next_cluster(&mut device, &boot, 0), Err(FsError::Corrupt));
        assert_eq!(next_cluster(&mut device, &boot, 1), Err(FsError::Corrupt));
    }

    #[test]
    fn fat12_chains_are_unsupported() {
        let boot = BootSector {
            fat_type: FatType::Fat12,
            ..fat16_boot()
        };
        let mut device = TestDevice::new(boot.total_sectors);
        assert_eq!(next_cluster(&mut device, &boot, 2), Err(FsError::Unsupported));
        assert_eq!(cluster_chain(&mut device, &boot, 2), Err(FsError::Unsupported));
    }

    #[test]
    fn device_errors_ride_out_as_io() {
        let boot = fat16_boot();
        let mut device = TestDevice::new(1);
        assert_eq!(
            next_cluster(&mut device, &boot, 2),
            Err(FsError::Io(IoError::Device))
        );
    }
}
