use system::file::{File, FileError};
use alloc::collections::VecDeque;

pub struct Pipe {
    buffer: VecDeque<u8>,
}

impl Pipe {
    pub fn new() -> Pipe {
        Pipe {
            buffer: VecDeque::new(),
        }
    }
}

impl File for Pipe {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, FileError> {
        let bytes_to_read = core::cmp::min(buf.len(), self.buffer.len());
        for i in 0..bytes_to_read {
            buf[i] = self.buffer.pop_front().unwrap();
        }
        Ok(bytes_to_read)
    }

    fn write(&mut self, buf: &[u8]) -> Result<usize, FileError> {
        for byte in buf {
            self.buffer.push_back(*byte);
        }
        Ok(buf.len())
    }
}
