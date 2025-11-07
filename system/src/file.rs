
use alloc::string::String;
use alloc::vec::Vec;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileError {
    NotFound,
    InvalidDescriptor,
    ReadError,
    WriteError,
    PermissionDenied,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileDescriptor(pub u64);

impl From<u64> for FileDescriptor {
    fn from(value: u64) -> Self {
        Self(value)
    }
}

impl Into<u64> for FileDescriptor {
    fn into(self) -> u64 {
        self.0
    }
}

pub trait File {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, FileError>;
    fn write(&mut self, buf: &[u8]) -> Result<usize, FileError>;
}
