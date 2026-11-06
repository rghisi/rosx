use block_protocol::client::{Block, Geometry};
use rxfat::boot::SECTOR_BYTES;
use rxfs::device::BlockDevice;
use rxfs::error::IoError;
use rxfs::spec::BLOCK_BYTES;

pub struct RemoteBlock {
    client: Block,
    device: u32,
    blocks: u64,
    device_blocks_per_block: u64,
}

pub struct FatSectors {
    client: Block,
    device: u32,
    sectors: u64,
    device_block_size: u64,
}

pub fn device_blocks_per_block(device_block_size: u32) -> Option<u64> {
    let device_block_size = u64::from(device_block_size);
    if device_block_size == 0 || BLOCK_BYTES as u64 % device_block_size != 0 {
        return None;
    }
    Some(BLOCK_BYTES as u64 / device_block_size)
}

pub fn fat_sector_count(block_count: u64, block_size: u32) -> Option<u64> {
    let block_size = u64::from(block_size);
    if block_size < u64::from(SECTOR_BYTES) || block_size % u64::from(SECTOR_BYTES) != 0 {
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

impl FatSectors {
    pub fn new(client: Block, device: u32, geometry: &Geometry) -> Option<FatSectors> {
        let sectors = fat_sector_count(geometry.block_count, geometry.block_size)?;
        Some(FatSectors {
            client,
            device,
            sectors,
            device_block_size: u64::from(geometry.block_size),
        })
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
            sector_window(sector, self.device_block_size).ok_or(IoError::Device)?;
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
}
