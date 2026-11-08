use alloc::string::String;
use block_protocol::manager::client::{Geometry, Manager};
use fs_core::device::BlockDevice;
use fs_core::error::IoError;

pub struct RemoteWindow {
    client: Manager,
    name: String,
    unit_bytes: u64,
    units: u64,
    device_blocks_per_unit: u64,
}

impl RemoteWindow {
    pub fn new(
        client: Manager,
        name: &str,
        geometry: &Geometry,
        unit_bytes: u64,
    ) -> Result<RemoteWindow, Manager> {
        let Some(device_blocks_per_unit) = Self::device_blocks_per_unit(unit_bytes, u64::from(geometry.block_size)) else {
            return Err(client);
        };
        Ok(RemoteWindow {
            client,
            name: String::from(name),
            unit_bytes,
            units: geometry.block_count / device_blocks_per_unit,
            device_blocks_per_unit,
        })
    }

    fn device_blocks_per_unit(unit_bytes: u64, device_block_size: u64) -> Option<u64> {
        if device_block_size == 0 || unit_bytes == 0 || unit_bytes % device_block_size != 0 {
            return None;
        }
        Some(unit_bytes / device_block_size)
    }

    pub fn into_client(self) -> Manager {
        self.client
    }

    fn device_block(&self, unit: u64) -> Option<u64> {
        if unit >= self.units {
            return None;
        }
        Some(unit * self.device_blocks_per_unit)
    }
}

impl BlockDevice for RemoteWindow {
    fn block_count(&self) -> u64 {
        self.units
    }

    fn read_block(&mut self, unit: u64, buf: &mut [u8]) -> Result<(), IoError> {
        if buf.len() as u64 != self.unit_bytes {
            return Err(IoError::Device);
        }
        let device_block = self.device_block(unit).ok_or(IoError::Device)?;
        let data = self
            .client
            .read_blocks(&self.name, device_block, self.device_blocks_per_unit as usize)
            .map_err(|_| IoError::Device)?;
        if data.len() as u64 != self.unit_bytes {
            return Err(IoError::Device);
        }
        buf.copy_from_slice(&data);
        Ok(())
    }

    fn write_block(&mut self, unit: u64, buf: &[u8]) -> Result<(), IoError> {
        if buf.len() as u64 != self.unit_bytes {
            return Err(IoError::Device);
        }
        let device_block = self.device_block(unit).ok_or(IoError::Device)?;
        self.client
            .write_blocks(
                &self.name,
                device_block,
                self.device_blocks_per_unit as usize,
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
    fn rxfs_window_divides_the_device_block() {
        assert_eq!(RemoteWindow::device_blocks_per_unit(4096, 512), Some(8));
        assert_eq!(RemoteWindow::device_blocks_per_unit(4096, 1024), Some(4));
        assert_eq!(RemoteWindow::device_blocks_per_unit(4096, 4096), Some(1));
        assert_eq!(RemoteWindow::device_blocks_per_unit(4096, 0), None);
        assert_eq!(RemoteWindow::device_blocks_per_unit(4096, 33), None);
        assert_eq!(RemoteWindow::device_blocks_per_unit(4096, 3072), None);
    }

    #[test]
    fn fat_window_demands_a_matching_device() {
        assert_eq!(RemoteWindow::device_blocks_per_unit(512, 512), Some(1));
        assert_eq!(RemoteWindow::device_blocks_per_unit(512, 1024), None);
        assert_eq!(RemoteWindow::device_blocks_per_unit(0, 512), None);
    }
}
