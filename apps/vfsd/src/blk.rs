use block_protocol::client::{Block, Geometry};
use rxfat::boot::SECTOR_BYTES;
use rxfs::device::BlockDevice;
use rxfs::error::IoError;
use rxfs::spec::{BLOCK_BYTES, FS_MAGIC, SUPERBLOCK_BLOCK};

pub struct RemoteBlock {
    client: Block,
    device: u32,
    blocks: u64,
    device_blocks_per_block: u64,
}

pub struct FatSectors {
    client: Block,
    device: u32,
    base: u64,
    sectors: u64,
    device_block_size: u64,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Partition {
    pub start: u64,
    pub sectors: u64,
}

const MBR_SIGNATURE_OFFSET: usize = 0x1FE;
const MBR_FIRST_ENTRY: usize = 0x1BE;
const FAT_PARTITION_KINDS: [u8; 7] = [0x01, 0x04, 0x06, 0x0B, 0x0C, 0x0E, 0x0F];

pub fn first_partition(head: &[u8]) -> Option<Partition> {
    let signature = u16::from_le_bytes(
        head.get(MBR_SIGNATURE_OFFSET..MBR_SIGNATURE_OFFSET + 2)?
            .try_into()
            .ok()?,
    );
    if signature != 0xAA55 {
        return None;
    }
    let entry = head.get(MBR_FIRST_ENTRY..MBR_FIRST_ENTRY + 16)?;
    if !FAT_PARTITION_KINDS.contains(&entry[2]) {
        return None;
    }
    let start = u32::from_le_bytes(entry[8..12].try_into().ok()?) as u64;
    let sectors = u32::from_le_bytes(entry[12..16].try_into().ok()?) as u64;
    if start == 0 || sectors == 0 {
        return None;
    }
    Some(Partition { start, sectors })
}

pub fn partition_view(total_sectors: u64, partition: &Partition) -> Option<u64> {
    if partition.start >= total_sectors {
        return None;
    }
    Some((total_sectors - partition.start).min(partition.sectors))
}

pub fn device_blocks_per_block(device_block_size: u32) -> Option<u64> {
    let device_block_size = u64::from(device_block_size);
    if device_block_size == 0 || !(BLOCK_BYTES as u64).is_multiple_of(device_block_size) {
        return None;
    }
    Some(BLOCK_BYTES as u64 / device_block_size)
}

pub fn fat_sector_count(block_count: u64, block_size: u32) -> Option<u64> {
    let block_size = u64::from(block_size);
    if block_size < u64::from(SECTOR_BYTES) || !block_size.is_multiple_of(u64::from(SECTOR_BYTES)) {
        return None;
    }
    block_count.checked_mul(block_size / u64::from(SECTOR_BYTES))
}

pub fn sector_window(sector: u64, device_block_size: u64) -> Option<(u64, usize)> {
    if device_block_size < u64::from(SECTOR_BYTES)
        || device_block_size % u64::from(SECTOR_BYTES) != 0
    {
        return None;
    }
    let byte = sector.checked_mul(u64::from(SECTOR_BYTES))?;
    Some((byte / device_block_size, (byte % device_block_size) as usize))
}

pub fn probe_head_blocks(device_block_size: u32) -> usize {
    let Some(block_size) = usize::try_from(device_block_size)
        .ok()
        .filter(|size| *size > 0)
    else {
        return 1;
    };
    let window = SUPERBLOCK_BLOCK as usize * BLOCK_BYTES + FS_MAGIC.len();
    window.div_ceil(block_size).max(1)
}

impl FatSectors {
    pub fn new(client: Block, device: u32, geometry: &Geometry) -> Option<FatSectors> {
        let sectors = fat_sector_count(geometry.block_count, geometry.block_size)?;
        Some(FatSectors {
            client,
            device,
            base: 0,
            sectors,
            device_block_size: u64::from(geometry.block_size),
        })
    }

    pub fn new_view(
        client: Block,
        device: u32,
        geometry: &Geometry,
        partition: Option<Partition>,
    ) -> Option<FatSectors> {
        let total = fat_sector_count(geometry.block_count, geometry.block_size)?;
        match partition {
            None => Some(FatSectors {
                client,
                device,
                base: 0,
                sectors: total,
                device_block_size: u64::from(geometry.block_size),
            }),
            Some(partition) => {
                let sectors = partition_view(total, &partition)?;
                Some(FatSectors {
                    client,
                    device,
                    base: partition.start,
                    sectors,
                    device_block_size: u64::from(geometry.block_size),
                })
            }
        }
    }
}

impl rxfat::device::BlockDevice for FatSectors {
    fn block_count(&self) -> u64 {
        self.sectors
    }

    fn read_block(&mut self, sector: u64, buf: &mut [u8]) -> Result<(), IoError> {
        if buf.len() != usize::from(SECTOR_BYTES) || sector >= self.sectors {
            return Err(IoError::Device);
        }
        let (device_block, offset) =
            sector_window(sector + self.base, self.device_block_size).ok_or(IoError::Device)?;
        let data = self
            .client
            .read_blocks(self.device, device_block, 1)
            .map_err(|_| IoError::Device)?;
        if data.len() as u64 != self.device_block_size || offset + usize::from(SECTOR_BYTES) > data.len() {
            return Err(IoError::Device);
        }
        buf.copy_from_slice(&data[offset..offset + usize::from(SECTOR_BYTES)]);
        Ok(())
    }

    fn write_block(&mut self, sector: u64, buf: &[u8]) -> Result<(), IoError> {
        if buf.len() != usize::from(SECTOR_BYTES) || sector >= self.sectors {
            return Err(IoError::Device);
        }
        if self.device_block_size != u64::from(SECTOR_BYTES) || self.base != 0 {
            return Err(IoError::Device);
        }
        self.client
            .write_blocks(self.device, sector, 1, buf)
            .map_err(|_| IoError::Device)
    }

    fn flush(&mut self) -> Result<(), IoError> {
        self.client.flush(self.device).map_err(|_| IoError::Flush)
    }
}

impl RemoteBlock {
    pub fn new(client: Block, device: u32, geometry: &Geometry) -> Option<RemoteBlock> {
        let device_blocks_per_block = device_blocks_per_block(geometry.block_size)?;
        Some(RemoteBlock {
            client,
            device,
            blocks: geometry.block_count / device_blocks_per_block,
            device_blocks_per_block,
        })
    }

    pub fn into_client(self) -> Block {
        self.client
    }

    fn device_block(&self, block: u64) -> Option<u64> {
        if block >= self.blocks {
            return None;
        }
        Some(block * self.device_blocks_per_block)
    }
}

impl BlockDevice for RemoteBlock {
    fn block_count(&self) -> u64 {
        self.blocks
    }

    fn read_block(&mut self, block: u64, buf: &mut [u8]) -> Result<(), IoError> {
        if buf.len() != BLOCK_BYTES {
            return Err(IoError::Device);
        }
        let device_block = self.device_block(block).ok_or(IoError::Device)?;
        let data = self
            .client
            .read_blocks(
                self.device,
                device_block,
                self.device_blocks_per_block as usize,
            )
            .map_err(|_| IoError::Device)?;
        if data.len() != BLOCK_BYTES {
            return Err(IoError::Device);
        }
        buf.copy_from_slice(&data);
        Ok(())
    }

    fn write_block(&mut self, block: u64, buf: &[u8]) -> Result<(), IoError> {
        if buf.len() != BLOCK_BYTES {
            return Err(IoError::Device);
        }
        let device_block = self.device_block(block).ok_or(IoError::Device)?;
        self.client
            .write_blocks(
                self.device,
                device_block,
                self.device_blocks_per_block as usize,
                buf,
            )
            .map_err(|_| IoError::Device)
    }

    fn flush(&mut self) -> Result<(), IoError> {
        self.client.flush(self.device).map_err(|_| IoError::Flush)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fat_sector_count_maps_each_block_size() {
        assert_eq!(fat_sector_count(100, 512), Some(100));
        assert_eq!(fat_sector_count(100, 1024), Some(200));
        assert_eq!(fat_sector_count(100, 4096), Some(800));
    }

    #[test]
    fn fat_sector_count_rejects_undersized_zero_and_overflow() {
        assert_eq!(fat_sector_count(100, 256), None);
        assert_eq!(fat_sector_count(100, 0), None);
        assert_eq!(fat_sector_count(100, 3), None);
        assert_eq!(fat_sector_count(u64::MAX, 512), Some(u64::MAX));
        assert_eq!(fat_sector_count(u64::MAX, 4096), None);
    }

    #[test]
    fn sector_window_splits_byte_address_on_larger_device_blocks() {
        assert_eq!(sector_window(0, 512), Some((0, 0)));
        assert_eq!(sector_window(7, 512), Some((7, 0)));
        assert_eq!(sector_window(0, 4096), Some((0, 0)));
        assert_eq!(sector_window(8, 4096), Some((1, 0)));
        assert_eq!(sector_window(9, 4096), Some((1, 512)));
        assert_eq!(sector_window(15, 4096), Some((1, 3584)));
    }

    #[test]
    fn sector_window_rejects_blocks_smaller_than_a_sector() {
        assert_eq!(sector_window(0, 256), None);
        assert_eq!(sector_window(0, 0), None);
    }

    #[test]
    fn probe_head_always_covers_fat_bpb_and_rxfs_superblock_magic() {
        assert_eq!(probe_head_blocks(512), 9);
        assert_eq!(probe_head_blocks(1024), 5);
        assert_eq!(probe_head_blocks(4096), 2);
        assert_eq!(probe_head_blocks(8192), 1);
        assert_eq!(probe_head_blocks(256), 17);
        assert_eq!(probe_head_blocks(0), 1);
    }

    fn mbr(entry: [u8; 16]) -> Vec<u8> {
        let mut head = vec![0u8; 512];
        head[0x1FE] = 0x55;
        head[0x1FF] = 0xAA;
        head[0x1BE..0x1CE].copy_from_slice(&entry);
        head
    }

    fn lba_entry(kind: u8, start: u32, sectors: u32) -> [u8; 16] {
        let mut entry = [0u8; 16];
        entry[2] = kind;
        entry[8..12].copy_from_slice(&start.to_le_bytes());
        entry[12..16].copy_from_slice(&sectors.to_le_bytes());
        entry
    }

    #[test]
    fn era_style_fat16_partition_is_decoded() {
        let head = mbr(lba_entry(0x06, 2_048, 62_512));
        assert_eq!(
            first_partition(&head),
            Some(Partition {
                start: 2_048,
                sectors: 62_512
            })
        );
    }

    #[test]
    fn fat32_partition_kinds_are_decoded() {
        for kind in [0x0B, 0x0C] {
            let head = mbr(lba_entry(kind, 63, 4_000));
            assert_eq!(first_partition(&head).map(|p| p.start), Some(63));
        }
    }

    #[test]
    fn zero_table_with_mbr_signature_is_not_a_partition() {
        assert_eq!(first_partition(&mbr([0u8; 16])), None);
    }

    #[test]
    fn foreign_partition_kinds_and_empty_bounds_are_rejected() {
        assert_eq!(first_partition(&mbr(lba_entry(0x83, 2_048, 100))), None);
        assert_eq!(first_partition(&mbr(lba_entry(0x06, 0, 100))), None);
        assert_eq!(first_partition(&mbr(lba_entry(0x06, 2_048, 0))), None);
    }

    #[test]
    fn mbr_without_signature_is_not_a_partition() {
        let mut head = vec![0u8; 512];
        head[0x1BE..0x1CE].copy_from_slice(&lba_entry(0x06, 2_048, 100));
        assert_eq!(first_partition(&head), None);
        assert_eq!(first_partition(&[]), None);
        assert_eq!(first_partition(&vec![0u8; 511]), None);
    }

    #[test]
    fn partition_view_clamps_to_the_device_and_drops_runaways() {
        assert_eq!(
            partition_view(100_000, &Partition { start: 2_048, sectors: 62_512 }),
            Some(62_512)
        );
        assert_eq!(
            partition_view(3_000, &Partition { start: 2_048, sectors: 62_512 }),
            Some(952)
        );
        assert_eq!(partition_view(2_048, &Partition { start: 2_048, sectors: 10 }), None);
        assert_eq!(partition_view(0, &Partition { start: 1, sectors: 10 }), None);
    }
}
