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

pub const FSINFO_FREE_COUNT_OFFSET: usize = 0x1E8;
const FSINFO_BACKUP_DISTANCE: u64 = 13;

pub fn write_fat_entry<D: BlockDevice>(
    device: &mut D,
    boot: &BootSector,
    cluster: u32,
    value: u32,
) -> Result<(), FsError> {
    if cluster < 2 || cluster > boot.cluster_count + 1 {
        return Err(FsError::Corrupt);
    }
    let width = match boot.fat_type {
        FatType::Fat12 => return Err(FsError::Unsupported),
        FatType::Fat16 => 2usize,
        FatType::Fat32 => 4usize,
    };
    let entry_bytes = u64::from(cluster) * width as u64;
    let fat_sector = entry_bytes / u64::from(SECTOR_BYTES);
    let offset = (entry_bytes % u64::from(SECTOR_BYTES)) as usize;
    for copy in 0..boot.fat_count {
        let sector =
            boot.reserved_sectors + u64::from(copy) * boot.fat_sectors + fat_sector;
        let mut block = [0u8; SECTOR_BYTES as usize];
        device.read_block(sector, &mut block).map_err(FsError::Io)?;
        match boot.fat_type {
            FatType::Fat12 => return Err(FsError::Unsupported),
            FatType::Fat16 => {
                block[offset..offset + 2].copy_from_slice(&(value as u16).to_le_bytes());
            }
            FatType::Fat32 => {
                let existing = u32::from_le_bytes([
                    block[offset],
                    block[offset + 1],
                    block[offset + 2],
                    block[offset + 3],
                ]);
                let merged = (existing & !FAT32_VALUE_MASK) | (value & FAT32_VALUE_MASK);
                block[offset..offset + 4].copy_from_slice(&merged.to_le_bytes());
            }
        }
        device.write_block(sector, &block).map_err(FsError::Io)?;
    }
    Ok(())
}

pub fn count_free_clusters<D: BlockDevice>(
    device: &mut D,
    boot: &BootSector,
) -> Result<u32, FsError> {
    let width = match boot.fat_type {
        FatType::Fat12 => return Err(FsError::Unsupported),
        FatType::Fat16 => 2usize,
        FatType::Fat32 => 4usize,
    };
    let mut free = 0u32;
    let mut block = [0u8; SECTOR_BYTES as usize];
    for fat_sector in 0..boot.fat_sectors {
        let sector = boot.reserved_sectors + fat_sector;
        device.read_block(sector, &mut block).map_err(FsError::Io)?;
        let base_cluster =
            (fat_sector * u64::from(SECTOR_BYTES) / width as u64) as u32;
        for index in 0..(SECTOR_BYTES as usize / width) as u32 {
            let cluster = base_cluster + index;
            if cluster < 2 || cluster > boot.cluster_count + 1 {
                continue;
            }
            let offset = index as usize * width;
            let value = match boot.fat_type {
                FatType::Fat12 => return Err(FsError::Unsupported),
                FatType::Fat16 => u32::from(u16::from_le_bytes([
                    block[offset],
                    block[offset + 1],
                ])),
                FatType::Fat32 => u32::from_le_bytes([
                    block[offset],
                    block[offset + 1],
                    block[offset + 2],
                    block[offset + 3],
                ]) & FAT32_VALUE_MASK,
            };
            if value == 0 {
                free += 1;
            }
        }
    }
    Ok(free)
}

pub fn write_free_cluster_count<D: BlockDevice>(
    device: &mut D,
    boot: &BootSector,
    free_clusters: u32,
) -> Result<(), FsError> {
    if boot.fat_type != FatType::Fat32 || boot.fsinfo_sector == 0 {
        return Ok(());
    }
    for sector in [
        u64::from(boot.fsinfo_sector),
        u64::from(boot.fsinfo_sector) + FSINFO_BACKUP_DISTANCE,
    ] {
        if sector == 0 || sector >= boot.reserved_sectors {
            continue;
        }
        let mut block = [0u8; SECTOR_BYTES as usize];
        device.read_block(sector, &mut block).map_err(FsError::Io)?;
        block[FSINFO_FREE_COUNT_OFFSET..FSINFO_FREE_COUNT_OFFSET + 4]
            .copy_from_slice(&free_clusters.to_le_bytes());
        device.write_block(sector, &block).map_err(FsError::Io)?;
    }
    Ok(())
}

pub fn chain_end(boot: &BootSector) -> u32 {
    match boot.fat_type {
        FatType::Fat12 => FAT16_END,
        FatType::Fat16 => FAT16_END,
        FatType::Fat32 => FAT32_END,
    }
}

fn scan_free_clusters<D: BlockDevice>(
    device: &mut D,
    boot: &BootSector,
    count: u32,
) -> Result<Vec<u32>, FsError> {
    let width = match boot.fat_type {
        FatType::Fat12 => return Err(FsError::Unsupported),
        FatType::Fat16 => 2usize,
        FatType::Fat32 => 4usize,
    };
    let mut free: Vec<u32> = Vec::new();
    let mut block = [0u8; SECTOR_BYTES as usize];
    for fat_sector in 0..boot.fat_sectors {
        device
            .read_block(boot.reserved_sectors + fat_sector, &mut block)
            .map_err(FsError::Io)?;
        let base_cluster = (fat_sector * u64::from(SECTOR_BYTES) / width as u64) as u32;
        for index in 0..(SECTOR_BYTES as usize / width) as u32 {
            let cluster = base_cluster + index;
            if cluster < 2 || cluster > boot.cluster_count + 1 {
                continue;
            }
            let offset = index as usize * width;
            let value = match boot.fat_type {
                FatType::Fat12 => return Err(FsError::Unsupported),
                FatType::Fat16 => {
                    u32::from(u16::from_le_bytes([block[offset], block[offset + 1]]))
                }
                FatType::Fat32 => {
                    u32::from_le_bytes([
                        block[offset],
                        block[offset + 1],
                        block[offset + 2],
                        block[offset + 3],
                    ]) & FAT32_VALUE_MASK
                }
            };
            if value == 0 {
                free.push(cluster);
                if free.len() == count as usize {
                    return Ok(free);
                }
            }
        }
    }
    Err(FsError::NoSpace)
}

pub fn allocate_chain<D: BlockDevice>(
    device: &mut D,
    boot: &BootSector,
    count: u32,
) -> Result<u32, FsError> {
    if count == 0 {
        return Err(FsError::InvalidPath);
    }
    let free = scan_free_clusters(device, boot, count)?;
    let zero_block = [0u8; SECTOR_BYTES as usize];
    for cluster in &free {
        let first = boot
            .first_sector_of_cluster(*cluster)
            .ok_or(FsError::Corrupt)?;
        for sector in first..first + u64::from(boot.sectors_per_cluster) {
            device.write_block(sector, &zero_block).map_err(FsError::Io)?;
        }
    }
    let end = chain_end(boot);
    for index in 0..free.len() {
        let next = if index + 1 == free.len() {
            end
        } else {
            free[index + 1]
        };
        write_fat_entry(device, boot, free[index], next)?;
    }
    Ok(free[0])
}

pub fn append_chain<D: BlockDevice>(
    device: &mut D,
    boot: &BootSector,
    tail_cluster: u32,
    count: u32,
) -> Result<u32, FsError> {
    let free = scan_free_clusters(device, boot, count)?;
    let zero_block = [0u8; SECTOR_BYTES as usize];
    for cluster in &free {
        let first = boot
            .first_sector_of_cluster(*cluster)
            .ok_or(FsError::Corrupt)?;
        for sector in first..first + u64::from(boot.sectors_per_cluster) {
            device.write_block(sector, &zero_block).map_err(FsError::Io)?;
        }
    }
    write_fat_entry(device, boot, tail_cluster, free[0])?;
    let end = chain_end(boot);
    for index in 0..free.len() {
        let next = if index + 1 == free.len() {
            end
        } else {
            free[index + 1]
        };
        write_fat_entry(device, boot, free[index], next)?;
    }
    Ok(free[0])
}

pub fn free_chain<D: BlockDevice>(
    device: &mut D,
    boot: &BootSector,
    head: u32,
) -> Result<(), FsError> {
    if head < 2 {
        return Ok(());
    }
    let chain = cluster_chain(device, boot, head)?;
    for cluster in chain {
        write_fat_entry(device, boot, cluster, 0)?;
    }
    Ok(())
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

        fn write_block(&mut self, block: u64, buf: &[u8]) -> Result<(), IoError> {
            let at = block as usize * SECTOR_BYTES as usize;
            if buf.len() != SECTOR_BYTES as usize {
                return Err(IoError::Device);
            }
            let Some(region) = self.bytes.get_mut(at..at + SECTOR_BYTES as usize) else {
                return Err(IoError::Device);
            };
            region.copy_from_slice(buf);
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
            fsinfo_sector: 0,
        }
    }

    pub fn fat32_boot() -> BootSector {
        BootSector {
            fat_type: FatType::Fat32,
            cluster_count: 100,
            fsinfo_sector: 1,
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

    pub fn slot_bytes(
        name: &[u8; 8],
        extension: &[u8; 3],
        attrs: u8,
        cluster: u16,
        size: u32,
    ) -> [u8; 32] {
        let mut slot = [b' '; 32];
        slot[..8].copy_from_slice(name);
        slot[8..11].copy_from_slice(extension);
        slot[0x0B] = attrs;
        slot[0x14..0x16].copy_from_slice(&0u16.to_le_bytes());
        slot[0x1A..0x1C].copy_from_slice(&cluster.to_le_bytes());
        slot[0x1C..0x20].copy_from_slice(&size.to_le_bytes());
        slot
    }

    pub fn put_slot(device: &mut TestDevice, sector: u64, slot_index: usize, slot: &[u8]) {
        let at = sector as usize * 512 + slot_index * crate::dir::ENTRY_BYTES;
        device.bytes[at..at + slot.len()].copy_from_slice(slot);
    }

    pub fn fill_deleted(device: &mut TestDevice, sector: u64) {
        let at = sector as usize * 512;
        device.bytes[at..at + 512].fill(0xE5);
    }

    pub fn install_boot(device: &mut TestDevice, boot: &BootSector) {
        let put16 = |bytes: &mut [u8], offset: usize, value: u16| {
            bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes())
        };
        let put32 = |bytes: &mut [u8], offset: usize, value: u32| {
            bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes())
        };
        put16(&mut device.bytes, 0x0B, boot.bytes_per_sector);
        device.bytes[0x0D] = boot.sectors_per_cluster as u8;
        put16(&mut device.bytes, 0x0E, boot.reserved_sectors as u16);
        device.bytes[0x10] = boot.fat_count as u8;
        let root_entries = match boot.root {
            crate::boot::RootLocation::Sectors { entries, .. } => entries,
            crate::boot::RootLocation::Cluster(_) => 0,
        };
        put16(&mut device.bytes, 0x11, root_entries as u16);
        if boot.total_sectors <= u64::from(u16::MAX) {
            put16(&mut device.bytes, 0x13, boot.total_sectors as u16);
        }
        put32(&mut device.bytes, 0x20, boot.total_sectors as u32);
        match boot.fat_type {
            FatType::Fat12 => {
                put16(&mut device.bytes, 0x16, boot.fat_sectors as u16);
                device.bytes[0x36..0x3E].copy_from_slice(b"FAT12   ");
            }
            FatType::Fat16 => {
                put16(&mut device.bytes, 0x16, boot.fat_sectors as u16);
                device.bytes[0x36..0x3E].copy_from_slice(b"FAT16   ");
            }
            FatType::Fat32 => {
                let RootLocation::Cluster(root_cluster) = boot.root else {
                    panic!("fat32 fixtures carry a cluster root");
                };
                put32(&mut device.bytes, 0x24, boot.fat_sectors as u32);
                put32(&mut device.bytes, 0x2C, root_cluster);
                device.bytes[0x5A..0x62].copy_from_slice(b"FAT32   ");
            }
        }
        device.bytes[0x1FE] = 0x55;
        device.bytes[0x1FF] = 0xAA;
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

    #[test]
    fn write_fat_entry_updates_both_copies() {
        let boot = fat16_boot();
        let mut device = TestDevice::new(boot.total_sectors);
        write_fat_entry(&mut device, &boot, 5, 7).expect("write");
        let (sector, offset) = fat16_entry_at(5);
        assert_eq!(
            u16::from_le_bytes([device.bytes[sector as usize * 512 + offset], device.bytes[sector as usize * 512 + offset + 1]]),
            7
        );
        let copy_one = sector as usize * 512 + offset + boot.fat_sectors as usize * 512;
        assert_eq!(
            u16::from_le_bytes([device.bytes[copy_one], device.bytes[copy_one + 1]]),
            7
        );
    }

    #[test]
    fn write_fat_entry_preserves_fat32_reserved_bits() {
        let boot = BootSector {
            reserved_sectors: 32,
            fat_type: FatType::Fat32,
            ..fat16_boot()
        };
        let mut device = TestDevice::new(boot.total_sectors);
        let entry_byte = 3u64 * 4;
        let sector = (boot.reserved_sectors + entry_byte / 512) as usize;
        device.bytes[sector * 512 + 12..sector * 512 + 16]
            .copy_from_slice(&0xFFFF_FFF8u32.to_le_bytes());
        write_fat_entry(&mut device, &boot, 3, 9).expect("write");
        let stored = u32::from_le_bytes([
            device.bytes[sector * 512 + 12],
            device.bytes[sector * 512 + 13],
            device.bytes[sector * 512 + 14],
            device.bytes[sector * 512 + 15],
        ]);
        assert_eq!(stored, 0xF000_0009);
    }

    #[test]
    fn write_fat_entry_refuses_clusters_outside_the_data_area() {
        let boot = fat16_boot();
        let mut device = TestDevice::new(boot.total_sectors);
        assert_eq!(write_fat_entry(&mut device, &boot, 0, 3), Err(FsError::Corrupt));
        assert_eq!(write_fat_entry(&mut device, &boot, 1, 3), Err(FsError::Corrupt));
        assert_eq!(
            write_fat_entry(&mut device, &boot, boot.cluster_count + 2, 3),
            Err(FsError::Corrupt)
        );
    }

    #[test]
    fn count_free_clusters_matches_the_allocations() {
        let boot = fat16_boot();
        let mut device = TestDevice::new(boot.total_sectors);
        assert_eq!(count_free_clusters(&mut device, &boot).expect("count"), 2_046);
        write_fat_entry(&mut device, &boot, 2, 3).expect("link");
        write_fat_entry(&mut device, &boot, 3, 0xFFF8).expect("end");
        write_fat_entry(&mut device, &boot, 2_047, 0xFFF8).expect("top");
        assert_eq!(count_free_clusters(&mut device, &boot).expect("recount"), 2_043);
    }

    #[test]
    fn free_count_upkeep_reaches_fsinfo_and_backup_only_on_fat32() {
        let fat16 = fat16_boot();
        let mut device = TestDevice::new(fat16.total_sectors);
        write_free_cluster_count(&mut device, &fat16, 1234).expect("no-op");
        assert!(device.bytes.iter().all(|byte| *byte == 0));

        let fat32 = BootSector {
            reserved_sectors: 32,
            fat_type: FatType::Fat32,
            fsinfo_sector: 1,
            ..fat16_boot()
        };
        let mut device = TestDevice::new(fat32.total_sectors);
        write_free_cluster_count(&mut device, &fat32, 4321).expect("update");
        for sector in [1u64, 14] {
            let at = sector as usize * 512 + 0x1E8;
            assert_eq!(
                u32::from_le_bytes([
                    device.bytes[at],
                    device.bytes[at + 1],
                    device.bytes[at + 2],
                    device.bytes[at + 3]
                ]),
                4321,
                "sector {sector}"
            );
        }
    }

    fn fat_entry_value(device: &TestDevice, cluster: u32) -> u32 {
        let (sector, offset) = fat16_entry_at(cluster);
        u16::from_le_bytes([
            device.bytes[sector as usize * 512 + offset],
            device.bytes[sector as usize * 512 + offset + 1],
        ]) as u32
    }

    #[test]
    fn allocate_chain_links_zeroes_and_refuses_exhaustion() {
        let boot = fat16_boot();
        let mut device = TestDevice::new(boot.total_sectors);
        for byte in device.bytes[49 * 512..52 * 512].iter_mut() {
            *byte = 0xAA;
        }
        assert_eq!(allocate_chain(&mut device, &boot, 3).expect("head"), 2);
        assert_eq!(fat_entry_value(&device, 2), 3);
        assert_eq!(fat_entry_value(&device, 3), 4);
        assert_eq!(fat_entry_value(&device, 4), 0xFFF8);
        let (sector, offset) = fat16_entry_at(4);
        let mirror = (sector as usize + boot.fat_sectors as usize) * 512 + offset;
        assert_eq!(
            u16::from_le_bytes([device.bytes[mirror], device.bytes[mirror + 1]]),
            0xFFF8
        );
        assert!(device.bytes[49 * 512..52 * 512].iter().all(|byte| *byte == 0));

        let fat_before = device.bytes[512..512 + 8 * 512].to_vec();
        assert_eq!(allocate_chain(&mut device, &boot, 3_000), Err(FsError::NoSpace));
        assert_eq!(device.bytes[512..512 + 8 * 512], fat_before[..]);
    }

    #[test]
    fn append_chain_extends_an_existing_one() {
        let boot = fat16_boot();
        let mut device = TestDevice::new(boot.total_sectors);
        allocate_chain(&mut device, &boot, 3).expect("initial three");
        append_chain(&mut device, &boot, 4, 2).expect("two more");
        assert_eq!(fat_entry_value(&device, 4), 5);
        assert_eq!(fat_entry_value(&device, 5), 6);
        assert_eq!(fat_entry_value(&device, 6), 0xFFF8);
        assert_eq!(
            cluster_chain(&mut device, &boot, 2).expect("walk"),
            vec![2, 3, 4, 5, 6]
        );
    }

    #[test]
    fn free_chain_clears_both_copies() {
        let boot = fat16_boot();
        let mut device = TestDevice::new(boot.total_sectors);
        allocate_chain(&mut device, &boot, 3).expect("allocate");
        free_chain(&mut device, &boot, 2).expect("free");
        for cluster in 2..5u32 {
            assert_eq!(fat_entry_value(&device, cluster), 0);
            let (sector, offset) = fat16_entry_at(cluster);
            let mirror = (sector as usize + boot.fat_sectors as usize) * 512 + offset;
            assert_eq!(u16::from_le_bytes([device.bytes[mirror], device.bytes[mirror + 1]]), 0);
        }
        free_chain(&mut device, &boot, 0).expect("empty head is a no-op");
    }

    #[test]
    fn zero_cluster_allocations_are_refused() {
        let boot = fat16_boot();
        let mut device = TestDevice::new(boot.total_sectors);
        assert_eq!(allocate_chain(&mut device, &boot, 0), Err(FsError::InvalidPath));
    }
}
