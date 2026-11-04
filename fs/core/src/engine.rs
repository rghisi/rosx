use crate::error::FsError;
use alloc::vec::Vec;
use core::fmt::Debug;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Regular,
    Directory,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirItem {
    pub kind: Kind,
    pub name: Vec<u8>,
}

pub trait MetadataView {
    fn kind(&self) -> Kind;
    fn size(&self) -> u64;
}

pub trait Engine {
    type Handle: Copy + Debug + PartialEq;
    type Metadata: MetadataView;

    fn lookup(&mut self, path_bytes: &[u8]) -> Result<Self::Metadata, FsError>;
    fn open(&mut self, path_bytes: &[u8]) -> Result<Self::Handle, FsError>;
    fn create(&mut self, path_bytes: &[u8]) -> Result<Self::Handle, FsError>;
    fn mkdir(&mut self, path_bytes: &[u8]) -> Result<(), FsError>;
    fn rmdir(&mut self, path_bytes: &[u8]) -> Result<(), FsError>;
    fn read(
        &mut self,
        handle: Self::Handle,
        offset: u64,
        buf: &mut [u8],
    ) -> Result<usize, FsError>;
    fn write(
        &mut self,
        handle: Self::Handle,
        offset: u64,
        data: &[u8],
    ) -> Result<usize, FsError>;
    fn stat(&mut self, handle: Self::Handle) -> Result<Self::Metadata, FsError>;
    fn close(&mut self, handle: Self::Handle) -> Result<(), FsError>;
    fn remove(&mut self, path_bytes: &[u8]) -> Result<(), FsError>;
    fn readdir(&mut self, path_bytes: &[u8]) -> Result<Vec<DirItem>, FsError>;
    fn sync(&mut self) -> Result<(), FsError>;
}
