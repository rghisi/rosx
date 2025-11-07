
use crate::file_arena::{FileArena, FileHandle};
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use system::file::{File, FileDescriptor, FileError};

struct InMemoryFile {
    path: String,
    content: Vec<u8>,
    cursor: usize,
}

impl File for InMemoryFile {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, FileError> {
        let bytes_to_read = core::cmp::min(buf.len(), self.content.len() - self.cursor);
        buf[..bytes_to_read].copy_from_slice(&self.content[self.cursor..self.cursor + bytes_to_read]);
        self.cursor += bytes_to_read;
        Ok(bytes_to_read)
    }

    fn write(&mut self, buf: &[u8]) -> Result<usize, FileError> {
        self.content.extend_from_slice(buf);
        Ok(buf.len())
    }
}

pub struct FileManager {
    files: Vec<InMemoryFile>,
    open_files: FileArena,
}

impl FileManager {
    pub fn new() -> Self {
        Self {
            files: Vec::new(),
            open_files: FileArena::new(10),
        }
    }

    pub fn open(&mut self, path: &str) -> Result<FileHandle, FileError> {
        let file_index = if let Some(index) = self.files.iter().position(|f| f.path == path) {
            index
        } else {
            let new_file = InMemoryFile {
                path: path.to_string(),
                content: Vec::new(),
                cursor: 0,
            };
            self.files.push(new_file);
            self.files.len() - 1
        };
        let fd = FileDescriptor(file_index as u64);
        self.open_files.add((file_index, fd)).map_err(|_| FileError::InvalidDescriptor)
    }

    pub fn close(&mut self, handle: FileHandle) -> Result<(), FileError> {
        self.open_files.remove(handle).map(|_| ()).map_err(|_| FileError::InvalidDescriptor)
    }

    pub fn read(
        &mut self,
        handle: FileHandle,
        buf: &mut [u8],
    ) -> Result<usize, FileError> {
        if let Ok((file_index, _)) = self.open_files.borrow(handle) {
            self.files[*file_index].read(buf)
        } else {
            Err(FileError::InvalidDescriptor)
        }
    }

    pub fn write(
        &mut self,
        handle: FileHandle,
        buf: &[u8],
    ) -> Result<usize, FileError> {
        if let Ok((file_index, _)) = self.open_files.borrow(handle) {
            self.files[*file_index].write(buf)
        } else {
            Err(FileError::InvalidDescriptor)
        }
    }
}
