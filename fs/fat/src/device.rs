use fs_core::error::IoError;

pub trait BlockDevice {
    fn block_count(&self) -> u64;
    fn read_block(&mut self, block: u64, buf: &mut [u8]) -> Result<(), IoError>;
}
