use alloc::string::String;
use block_protocol::manager::client::{Geometry, Manager};
use fs_core::device::BlockDevice;
use rxfs::error::IoError;
use rxfs::spec::BLOCK_BYTES;

pub struct RemoteBlock {
    client: Manager,
    name: String,
    blocks: u64,
    device_blocks_per_block: u64,
}

impl RemoteBlock {
    pub fn new(client: Manager, name: &str, geometry: &Geometry) -> Result<RemoteBlock, Manager> {
        let Some(device_blocks_per_block) = Self::device_blocks_per_block(geometry.block_size)
        else {
            return Err(client);
        };
        Ok(RemoteBlock {
            client,
            name: String::from(name),
            blocks: geometry.block_count / device_blocks_per_block,
            device_blocks_per_block,
        })
    }

    fn device_blocks_per_block(device_block_size: u32) -> Option<u64> {
        let device_block_size = u64::from(device_block_size);
        if device_block_size == 0 || BLOCK_BYTES as u64 % device_block_size != 0 {
            return None;
        }
        Some(BLOCK_BYTES as u64 / device_block_size)
    }

    pub fn into_client(self) -> Manager {
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
            .read_blocks(&self.name, device_block, self.device_blocks_per_block as usize)
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
                &self.name,
                device_block,
                self.device_blocks_per_block as usize,
                buf,
            )
            .map_err(|_| IoError::Device)
    }

    fn flush(&mut self) -> Result<(), IoError> {
        self.client.flush(&self.name).map_err(|_| IoError::Flush)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_blocks_per_block_divides_the_filesystem_block() {
        assert_eq!(RemoteBlock::device_blocks_per_block(512), Some(8));
        assert_eq!(RemoteBlock::device_blocks_per_block(1024), Some(4));
        assert_eq!(RemoteBlock::device_blocks_per_block(4096), Some(1));
        assert_eq!(RemoteBlock::device_blocks_per_block(0), None);
        assert_eq!(RemoteBlock::device_blocks_per_block(33), None);
        assert_eq!(RemoteBlock::device_blocks_per_block(3072), None);
    }
}
