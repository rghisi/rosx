use crate::error::IoError;

pub trait BlockDevice {
    fn block_count(&self) -> u64;
    fn read_block(&mut self, block: u64, buf: &mut [u8]) -> Result<(), IoError>;
    fn write_block(&mut self, block: u64, buf: &[u8]) -> Result<(), IoError>;
    fn flush(&mut self) -> Result<(), IoError>;

    fn read_blocks(&mut self, first: u64, count: usize, dest: &mut [u8]) -> Result<(), IoError> {
        let unit = run_unit(count, dest.len()).ok_or(IoError::Device)?;
        for (index, chunk) in dest.chunks_exact_mut(unit).enumerate() {
            let block = first.checked_add(index as u64).ok_or(IoError::Device)?;
            self.read_block(block, chunk)?;
        }
        Ok(())
    }

    fn write_blocks(&mut self, first: u64, count: usize, data: &[u8]) -> Result<(), IoError> {
        let unit = run_unit(count, data.len()).ok_or(IoError::Device)?;
        for (index, chunk) in data.chunks_exact(unit).enumerate() {
            let block = first.checked_add(index as u64).ok_or(IoError::Device)?;
            self.write_block(block, chunk)?;
        }
        Ok(())
    }
}

pub fn run_unit(count: usize, bytes: usize) -> Option<usize> {
    if count == 0 || bytes == 0 || bytes % count != 0 {
        return None;
    }
    Some(bytes / count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec::Vec;

    struct Mock {
        unit_bytes: usize,
        blocks: u64,
        fail_at: Option<u64>,
        read_log: Vec<u64>,
        write_log: Vec<(u64, usize)>,
    }

    impl Mock {
        fn new(blocks: u64) -> Self {
            Mock {
                unit_bytes: 4,
                blocks,
                fail_at: None,
                read_log: Vec::new(),
                write_log: Vec::new(),
            }
        }
    }

    impl BlockDevice for Mock {
        fn block_count(&self) -> u64 {
            self.blocks
        }

        fn read_block(&mut self, block: u64, buf: &mut [u8]) -> Result<(), IoError> {
            if block >= self.blocks || buf.len() != self.unit_bytes {
                return Err(IoError::Device);
            }
            self.read_log.push(block);
            if self.fail_at == Some(block) {
                return Err(IoError::Device);
            }
            buf.fill(block as u8);
            Ok(())
        }

        fn write_block(&mut self, block: u64, buf: &[u8]) -> Result<(), IoError> {
            if block >= self.blocks || buf.len() != self.unit_bytes {
                return Err(IoError::Device);
            }
            self.write_log.push((block, buf.len()));
            Ok(())
        }

        fn flush(&mut self) -> Result<(), IoError> {
            Ok(())
        }
    }

    #[test]
    fn the_default_read_run_equals_per_unit_reads() {
        let mut mock = Mock::new(8);
        let mut dest = [0u8; 12];
        mock.read_blocks(2, 3, &mut dest).unwrap();
        assert_eq!(mock.read_log, alloc::vec![2, 3, 4]);
        assert_eq!(dest, [2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4]);
    }

    #[test]
    fn the_default_write_run_equals_per_unit_writes() {
        let mut mock = Mock::new(8);
        let data = [9u8; 12];
        mock.write_blocks(5, 3, &data).unwrap();
        assert_eq!(mock.write_log, alloc::vec![(5, 4), (6, 4), (7, 4)]);
    }

    #[test]
    fn a_run_that_fails_midway_reports_the_device_error() {
        let mut mock = Mock::new(8);
        mock.fail_at = Some(4);
        let mut dest = [0u8; 12];
        assert_eq!(mock.read_blocks(2, 3, &mut dest), Err(IoError::Device));
        assert_eq!(mock.read_log, alloc::vec![2, 3, 4]);
    }

    #[test]
    fn a_run_must_divide_its_buffer_exactly() {
        let mut mock = Mock::new(8);
        let mut dest = [0u8; 8];
        assert_eq!(mock.read_blocks(0, 0, &mut dest), Err(IoError::Device));
        assert_eq!(mock.read_blocks(0, 2, &mut []), Err(IoError::Device));
        assert_eq!(mock.read_blocks(0, 3, &mut dest), Err(IoError::Device));
        assert_eq!(mock.write_blocks(0, 3, &dest), Err(IoError::Device));
        assert!(mock.read_log.is_empty());
        assert!(mock.write_log.is_empty());
    }

    #[test]
    fn a_run_overflowing_the_block_counter_is_refused_before_any_access() {
        let mut mock = Mock::new(8);
        let mut dest = [0u8; 8];
        assert_eq!(
            mock.read_blocks(u64::MAX - 1, 4, &mut dest),
            Err(IoError::Device)
        );
        assert!(mock.read_log.is_empty());
    }

    #[test]
    fn run_unit_demands_a_positive_exactly_dividing_count() {
        assert_eq!(run_unit(4, 2048), Some(512));
        assert_eq!(run_unit(1, 512), Some(512));
        assert_eq!(run_unit(0, 512), None);
        assert_eq!(run_unit(2, 0), None);
        assert_eq!(run_unit(3, 512), None);
        assert_eq!(run_unit(513, 512), None);
    }
}
