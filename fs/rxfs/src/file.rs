use crate::device::BlockDevice;
use crate::error::IoError;
use crate::spec::BLOCK_BYTES;
#[cfg(feature = "std")] use std::fs::{File, OpenOptions};
#[cfg(feature = "std")] use std::io::{Read, Seek, SeekFrom, Write};
#[cfg(feature = "std")] use std::path::Path;

pub struct FileDevice {
    file: File,
    blocks: u64,
}

impl FileDevice {
    pub fn create(path: &Path, blocks: u64) -> std::io::Result<FileDevice> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(path)?;
        file.set_len(blocks * BLOCK_BYTES as u64)?;
        Ok(FileDevice { file, blocks })
    }

    pub fn open(path: &Path) -> std::io::Result<FileDevice> {
        let file = OpenOptions::new().read(true).write(true).open(path)?;
        let blocks = file.metadata()?.len() / BLOCK_BYTES as u64;
        Ok(FileDevice { file, blocks })
    }
}

impl BlockDevice for FileDevice {
    fn block_count(&self) -> u64 {
        self.blocks
    }

    fn read_block(&mut self, block: u64, buf: &mut [u8]) -> Result<(), IoError> {
        if buf.len() != BLOCK_BYTES || block >= self.blocks {
            return Err(IoError::Device);
        }
        self.file
            .seek(SeekFrom::Start(block * BLOCK_BYTES as u64))
            .map_err(|_| IoError::Device)?;
        self.file.read_exact(buf).map_err(|_| IoError::Device)
    }

    fn write_block(&mut self, block: u64, data: &[u8]) -> Result<(), IoError> {
        if data.len() != BLOCK_BYTES || block >= self.blocks {
            return Err(IoError::Device);
        }
        self.file
            .seek(SeekFrom::Start(block * BLOCK_BYTES as u64))
            .map_err(|_| IoError::Device)?;
        self.file.write_all(data).map_err(|_| IoError::Device)
    }

    fn flush(&mut self) -> Result<(), IoError> {
        self.file.sync_all().map_err(|_| IoError::Flush)
    }
}
