use ata_protocol::client::Ata;
use ata_protocol::wire::SECTOR_BYTES;
use rxfs::device::BlockDevice;
use rxfs::error::IoError;
use rxfs::spec::BLOCK_BYTES;

const SECTORS_PER_BLOCK: u64 = (BLOCK_BYTES / SECTOR_BYTES) as u64;

pub struct AtaDisk {
    ata: Ata,
    drive: u8,
    blocks: u64,
}

impl AtaDisk {
    pub fn new(ata: Ata, drive: u8, sectors: u64) -> AtaDisk {
        AtaDisk {
            ata,
            drive,
            blocks: sectors / SECTORS_PER_BLOCK,
        }
    }

    pub fn into_ata(self) -> Ata {
        self.ata
    }

    fn block_lba(&self, block: u64) -> Option<u64> {
        if block >= self.blocks {
            return None;
        }
        Some(block * SECTORS_PER_BLOCK)
    }
}

impl BlockDevice for AtaDisk {
    fn block_count(&self) -> u64 {
        self.blocks
    }

    fn read_block(&mut self, block: u64, buf: &mut [u8]) -> Result<(), IoError> {
        if buf.len() != BLOCK_BYTES {
            return Err(IoError::Device);
        }
        let lba = self.block_lba(block).ok_or(IoError::Device)?;
        let data = self
            .ata
            .read(self.drive, lba, SECTORS_PER_BLOCK as usize)
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
        let lba = self.block_lba(block).ok_or(IoError::Device)?;
        self.ata
            .write(self.drive, lba, buf)
            .map_err(|_| IoError::Device)
    }

    fn flush(&mut self) -> Result<(), IoError> {
        self.ata.flush(self.drive).map_err(|_| IoError::Flush)
    }
}
