
use alloc::vec::Vec;
use alloc::collections::VecDeque;
use system::file::FileDescriptor;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct FileHandle {
    pub index: u8,
    pub generation: u8,
}

#[derive(Debug)]
pub enum Error {
    FileNotFound,
}

pub struct FileArena {
    files: Vec<Option<(usize, FileDescriptor)>>,
    generations: Vec<u8>,
    free_slots: VecDeque<u8>,
}

impl FileArena {
    pub fn new(initial_capacity: usize) -> Self {
        assert!(
            initial_capacity > 0 && initial_capacity <= 256,
            "FileArena capacity must be between 1 and 256"
        );

        let mut files = Vec::with_capacity(initial_capacity);
        let mut generations = Vec::with_capacity(initial_capacity);
        let mut free_slots = VecDeque::with_capacity(initial_capacity);
        for slot in 0..initial_capacity {
            files.push(None);
            generations.push(0);
            free_slots.push_back(slot as u8);
        }

        FileArena {
            files,
            generations,
            free_slots,
        }
    }

    pub fn add(&mut self, file: (usize, FileDescriptor)) -> Result<FileHandle, Error> {
        if self.free_slots.is_empty() {
            let increment = self.generations.capacity();
            let new_size = increment + increment;
            if new_size > 256 {
                panic!("FileArena capacity exceeded");
            }
            for i in increment..new_size {
                self.files.push(None);
                self.generations.push(0);
                self.free_slots.push_back(i as u8);
            }
        }

        let index = self.free_slots.pop_front().unwrap() as usize;
        let generation = self.generations[index];
        self.files[index] = Some(file);

        Ok(FileHandle {
            index: index as u8,
            generation,
        })
    }

    pub fn borrow(&self, handle: FileHandle) -> Result<&(usize, FileDescriptor), Error> {
        if self.generations[handle.index as usize] == handle.generation {
            return Ok(self.files[handle.index as usize].as_ref().unwrap());
        }

        Err(Error::FileNotFound)
    }

    pub fn remove(&mut self, handle: FileHandle) -> Result<(usize, FileDescriptor), Error> {
        let index = handle.index as usize;
        let generation = handle.generation;
        if generation == self.generations[index] {
            let file = self.files[handle.index as usize].take().unwrap();
            self.files[handle.index as usize] = None;
            let next_generation = generation + 1;
            self.generations[index] = next_generation;
            self.free_slots.push_back(index as u8);

            Ok(file)
        } else {
            Err(Error::FileNotFound)
        }
    }
}
