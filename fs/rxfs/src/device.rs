use crate::error::IoError;
use crate::spec::BLOCK_BYTES;
use alloc::vec;
use alloc::vec::Vec;
use core::ops::Range;

pub trait BlockDevice {
    fn block_count(&self) -> u64;
    fn read_block(&mut self, block: u64, buf: &mut [u8]) -> Result<(), IoError>;
    fn write_block(&mut self, block: u64, buf: &[u8]) -> Result<(), IoError>;
    fn flush(&mut self) -> Result<(), IoError>;
}

pub struct MemoryDevice {
    bytes: Vec<u8>,
}

impl MemoryDevice {
    pub fn new(blocks: u64) -> Self {
        let size = usize::try_from(blocks.saturating_mul(BLOCK_BYTES as u64)).unwrap_or(usize::MAX);
        Self {
            bytes: vec![0u8; size],
        }
    }
}

impl BlockDevice for MemoryDevice {
    fn block_count(&self) -> u64 {
        (self.bytes.len() / BLOCK_BYTES) as u64
    }

    fn read_block(&mut self, block: u64, buf: &mut [u8]) -> Result<(), IoError> {
        let range = block_range(block, buf.len(), self.bytes.len())?;
        buf.copy_from_slice(&self.bytes[range]);
        Ok(())
    }

    fn write_block(&mut self, block: u64, buf: &[u8]) -> Result<(), IoError> {
        let range = block_range(block, buf.len(), self.bytes.len())?;
        self.bytes[range].copy_from_slice(buf);
        Ok(())
    }

    fn flush(&mut self) -> Result<(), IoError> {
        Ok(())
    }
}

fn block_range(block: u64, len: usize, capacity: usize) -> Result<Range<usize>, IoError> {
    if len != BLOCK_BYTES {
        return Err(IoError::Device);
    }
    let index = usize::try_from(block).map_err(|_| IoError::Device)?;
    let start = index
        .checked_mul(BLOCK_BYTES)
        .ok_or(IoError::Device)?;
    let end = start.checked_add(BLOCK_BYTES).ok_or(IoError::Device)?;
    if end > capacity {
        return Err(IoError::Device);
    }
    Ok(start..end)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pattern_block(seed: u8) -> Vec<u8> {
        (0..BLOCK_BYTES).map(|i| (i as u8).wrapping_add(seed)).collect()
    }

    #[test]
    fn new_reports_block_count() {
        assert_eq!(MemoryDevice::new(0).block_count(), 0);
        assert_eq!(MemoryDevice::new(1).block_count(), 1);
        assert_eq!(MemoryDevice::new(17).block_count(), 17);
    }

    #[test]
    fn fresh_blocks_read_as_zero() {
        let mut dev = MemoryDevice::new(2);
        let mut buf = vec![0xFFu8; BLOCK_BYTES];
        dev.read_block(0, &mut buf).unwrap();
        assert!(buf.iter().all(|&b| b == 0));
        dev.read_block(1, &mut buf).unwrap();
        assert!(buf.iter().all(|&b| b == 0));
    }

    #[test]
    fn write_then_read_roundtrip() {
        let mut dev = MemoryDevice::new(3);
        let data = pattern_block(7);
        dev.write_block(2, &data).unwrap();
        let mut buf = vec![0u8; BLOCK_BYTES];
        dev.read_block(2, &mut buf).unwrap();
        assert_eq!(buf, data);
    }

    #[test]
    fn writes_are_confined_to_their_block() {
        let mut dev = MemoryDevice::new(3);
        dev.write_block(1, &pattern_block(3)).unwrap();
        let mut buf = vec![0xAAu8; BLOCK_BYTES];
        dev.read_block(0, &mut buf).unwrap();
        assert!(buf.iter().all(|&b| b == 0));
        dev.read_block(2, &mut buf).unwrap();
        assert!(buf.iter().all(|&b| b == 0));
    }

    #[test]
    fn rewrite_replaces_previous_content() {
        let mut dev = MemoryDevice::new(1);
        dev.write_block(0, &pattern_block(1)).unwrap();
        let zeros = vec![0u8; BLOCK_BYTES];
        dev.write_block(0, &zeros).unwrap();
        let mut buf = vec![0xFFu8; BLOCK_BYTES];
        dev.read_block(0, &mut buf).unwrap();
        assert_eq!(buf, zeros);
    }

    #[test]
    fn out_of_range_access_is_device_error() {
        let mut dev = MemoryDevice::new(2);
        let data = vec![0u8; BLOCK_BYTES];
        let mut buf = vec![0u8; BLOCK_BYTES];
        assert_eq!(dev.read_block(2, &mut buf), Err(IoError::Device));
        assert_eq!(dev.write_block(2, &data), Err(IoError::Device));
        assert_eq!(dev.read_block(u64::MAX, &mut buf), Err(IoError::Device));
        assert_eq!(dev.write_block(u64::MAX, &data), Err(IoError::Device));
    }

    #[test]
    fn empty_device_rejects_every_access() {
        let mut dev = MemoryDevice::new(0);
        let data = vec![0u8; BLOCK_BYTES];
        let mut buf = vec![0u8; BLOCK_BYTES];
        assert_eq!(dev.read_block(0, &mut buf), Err(IoError::Device));
        assert_eq!(dev.write_block(0, &data), Err(IoError::Device));
    }

    #[test]
    fn wrong_buffer_length_is_device_error() {
        let mut dev = MemoryDevice::new(1);
        let mut small = vec![0u8; BLOCK_BYTES - 1];
        let mut big = vec![0u8; BLOCK_BYTES + 1];
        assert_eq!(dev.read_block(0, &mut small), Err(IoError::Device));
        assert_eq!(dev.write_block(0, &small), Err(IoError::Device));
        assert_eq!(dev.read_block(0, &mut big[..]), Err(IoError::Device));
        assert_eq!(dev.write_block(0, &big), Err(IoError::Device));
        let empty: &mut [u8] = &mut [];
        assert_eq!(dev.read_block(0, empty), Err(IoError::Device));
    }

    #[test]
    fn flush_succeeds() {
        let mut dev = MemoryDevice::new(1);
        assert_eq!(dev.flush(), Ok(()));
    }
}
