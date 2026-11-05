use crate::error::FsError;
use crate::path::{self, PathParts};
use alloc::collections::BTreeMap;
use alloc::vec::Vec;
pub use fs_core::engine::{DirItem, Kind};
use fs_core::engine::{Engine, MetadataView};
pub use crate::path::{MAX_DEPTH, MAX_NAME_BYTES, MAX_PATH_BYTES};

pub const MAX_OPEN_FILES: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Handle {
    slot: u8,
    generation: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stat {
    pub kind: Kind,
    pub size: u64,
}

struct Entry {
    kind: Kind,
    bytes: Vec<u8>,
    generation: u32,
}

struct OpenSlot {
    key: Vec<u8>,
    file_generation: u32,
    open_generation: u32,
}

pub struct FileSystem {
    entries: BTreeMap<Vec<u8>, Entry>,
    opens: [Option<OpenSlot>; MAX_OPEN_FILES],
    next_generation: u32,
}

fn stat_of(entry: &Entry) -> Stat {
    Stat {
        kind: entry.kind,
        size: entry.bytes.len() as u64,
    }
}

fn child_name<'a>(key: &'a [u8], dir_key: &[u8]) -> Option<&'a [u8]> {
    match key.iter().rposition(|byte| *byte == b'/') {
        Some(index) => (&key[..index] == dir_key).then_some(&key[index + 1..]),
        None => dir_key.is_empty().then_some(key),
    }
}

impl FileSystem {
    #[allow(clippy::new_without_default)]
    pub fn new() -> FileSystem {
        FileSystem {
            entries: BTreeMap::new(),
            opens: core::array::from_fn(|_| None),
            next_generation: 1,
        }
    }

    pub fn sync(&mut self) -> Result<(), FsError> {
        Ok(())
    }

    fn resolve_parents(&self, parts: &PathParts<'_>) -> Result<(), FsError> {
        let mut parent = Vec::new();
        for depth in 0..parts.len().saturating_sub(1) {
            if depth > 0 {
                parent.push(b'/');
            }
            parent.extend_from_slice(parts.get(depth).ok_or(FsError::InvalidPath)?);
            match self.entries.get(&parent) {
                Some(entry) if entry.kind == Kind::Directory => {}
                Some(_) => return Err(FsError::NotADirectory),
                None => return Err(FsError::NotFound),
            }
        }
        Ok(())
    }

    pub fn lookup(&mut self, path_bytes: &[u8]) -> Result<Stat, FsError> {
        let parts = path::parse(path_bytes)?;
        if parts.is_root() {
            return Ok(Stat {
                kind: Kind::Directory,
                size: 0,
            });
        }
        self.resolve_parents(&parts)?;
        let entry = self.entries.get(&parts.key()).ok_or(FsError::NotFound)?;
        Ok(stat_of(entry))
    }

    pub fn open(&mut self, path_bytes: &[u8]) -> Result<Handle, FsError> {
        let parts = path::parse(path_bytes)?;
        if parts.is_root() {
            return Err(FsError::IsADirectory);
        }
        self.resolve_parents(&parts)?;
        let entry = self.entries.get(&parts.key()).ok_or(FsError::NotFound)?;
        if entry.kind == Kind::Directory {
            return Err(FsError::IsADirectory);
        }
        self.reserve_slot(parts.key(), entry.generation)
    }

    pub fn create(&mut self, path_bytes: &[u8]) -> Result<Handle, FsError> {
        let parts = path::parse(path_bytes)?;
        if parts.is_root() {
            return Err(FsError::AlreadyExists);
        }
        self.resolve_parents(&parts)?;
        let key = parts.key();
        if self.entries.contains_key(&key) {
            return Err(FsError::AlreadyExists);
        }
        let file_generation = self.take_generation();
        let handle = self.reserve_slot(key.clone(), file_generation)?;
        self.entries.insert(
            key,
            Entry {
                kind: Kind::Regular,
                bytes: Vec::new(),
                generation: file_generation,
            },
        );
        Ok(handle)
    }

    pub fn read(&mut self, handle: Handle, offset: u64, buf: &mut [u8]) -> Result<usize, FsError> {
        let file = self.live_file(handle)?;
        let size = file.bytes.len() as u64;
        if offset >= size {
            return Ok(0);
        }
        let start = offset as usize;
        let copied = (file.bytes.len() - start).min(buf.len());
        buf[..copied].copy_from_slice(&file.bytes[start..start + copied]);
        Ok(copied)
    }

    pub fn write(&mut self, handle: Handle, offset: u64, data: &[u8]) -> Result<usize, FsError> {
        let file = self.live_file(handle)?;
        if data.is_empty() {
            return Ok(0);
        }
        let start = usize::try_from(offset).unwrap_or(usize::MAX);
        let end = usize::try_from(offset.saturating_add(data.len() as u64)).unwrap_or(usize::MAX);
        if file.bytes.len() < end {
            file.bytes.resize(end, 0);
        }
        file.bytes[start..end].copy_from_slice(data);
        Ok(data.len())
    }

    pub fn stat(&mut self, handle: Handle) -> Result<Stat, FsError> {
        let file = self.live_file(handle)?;
        Ok(Stat {
            kind: Kind::Regular,
            size: file.bytes.len() as u64,
        })
    }

    pub fn close(&mut self, handle: Handle) -> Result<(), FsError> {
        Self::live_open(&self.opens, handle)?;
        self.opens[handle.slot as usize] = None;
        Ok(())
    }

    pub fn remove(&mut self, path_bytes: &[u8]) -> Result<(), FsError> {
        let parts = path::parse(path_bytes)?;
        if parts.is_root() {
            return Err(FsError::InvalidPath);
        }
        self.resolve_parents(&parts)?;
        self.entries
            .remove(&parts.key())
            .map(|_| ())
            .ok_or(FsError::NotFound)
    }

    pub fn readdir(&mut self, path_bytes: &[u8]) -> Result<Vec<DirItem>, FsError> {
        let parts = path::parse(path_bytes)?;
        let dir_key = if parts.is_root() {
            Vec::new()
        } else {
            self.resolve_parents(&parts)?;
            let entry = self.entries.get(&parts.key()).ok_or(FsError::NotFound)?;
            if entry.kind != Kind::Directory {
                return Err(FsError::NotADirectory);
            }
            parts.key()
        };
        Ok(self
            .entries
            .iter()
            .filter_map(|(key, entry)| {
                let name = child_name(key, &dir_key)?;
                Some(DirItem {
                    kind: entry.kind,
                    name: name.to_vec(),
                })
            })
            .collect())
    }

    fn take_generation(&mut self) -> u32 {
        let generation = self.next_generation;
        self.next_generation += 1;
        generation
    }

    fn reserve_slot(&mut self, key: Vec<u8>, file_generation: u32) -> Result<Handle, FsError> {
        let open_generation = self.take_generation();
        for (index, entry) in self.opens.iter_mut().enumerate() {
            if entry.is_none() {
                *entry = Some(OpenSlot {
                    key,
                    file_generation,
                    open_generation,
                });
                return Ok(Handle {
                    slot: index as u8,
                    generation: open_generation,
                });
            }
        }
        Err(FsError::TooManyOpen)
    }

    fn live_open(opens: &[Option<OpenSlot>], handle: Handle) -> Result<&OpenSlot, FsError> {
        opens
            .get(handle.slot as usize)
            .and_then(|entry| entry.as_ref())
            .filter(|entry| entry.open_generation == handle.generation)
            .ok_or(FsError::BadHandle)
    }

    fn live_file(&mut self, handle: Handle) -> Result<&mut Entry, FsError> {
        let Self {
            entries,
            opens,
            next_generation: _,
        } = self;
        let slot = Self::live_open(opens, handle)?;
        let entry = entries.get_mut(&slot.key).ok_or(FsError::BadHandle)?;
        if entry.generation != slot.file_generation {
            return Err(FsError::BadHandle);
        }
        Ok(entry)
    }
}

impl MetadataView for Stat {
    fn kind(&self) -> Kind {
        self.kind
    }

    fn size(&self) -> u64 {
        self.size
    }
}

impl Engine for FileSystem {
    type Handle = Handle;
    type Metadata = Stat;

    fn lookup(&mut self, path_bytes: &[u8]) -> Result<Self::Metadata, FsError> {
        FileSystem::lookup(self, path_bytes)
    }

    fn open(&mut self, path_bytes: &[u8]) -> Result<Self::Handle, FsError> {
        FileSystem::open(self, path_bytes)
    }

    fn create(&mut self, path_bytes: &[u8]) -> Result<Self::Handle, FsError> {
        FileSystem::create(self, path_bytes)
    }

    fn mkdir(&mut self, _path_bytes: &[u8]) -> Result<(), FsError> {
        Err(FsError::Unsupported)
    }

    fn rmdir(&mut self, _path_bytes: &[u8]) -> Result<(), FsError> {
        Err(FsError::Unsupported)
    }

    fn read(
        &mut self,
        handle: Self::Handle,
        offset: u64,
        buf: &mut [u8],
    ) -> Result<usize, FsError>
    {
        FileSystem::read(self, handle, offset, buf)
    }

    fn write(
        &mut self,
        handle: Self::Handle,
        offset: u64,
        data: &[u8],
    ) -> Result<usize, FsError>
    {
        FileSystem::write(self, handle, offset, data)
    }

    fn stat(&mut self, handle: Self::Handle) -> Result<Self::Metadata, FsError> {
        FileSystem::stat(self, handle)
    }

    fn close(&mut self, handle: Self::Handle) -> Result<(), FsError> {
        FileSystem::close(self, handle)
    }

    fn remove(&mut self, path_bytes: &[u8]) -> Result<(), FsError> {
        FileSystem::remove(self, path_bytes)
    }

    fn readdir(&mut self, path_bytes: &[u8]) -> Result<Vec<DirItem>, FsError> {
        FileSystem::readdir(self, path_bytes)
    }

    fn sync(&mut self) -> Result<(), FsError> {
        FileSystem::sync(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::format;
    use alloc::string::String;
    use alloc::vec;

    fn create(fs: &mut FileSystem, name: &str) -> Handle {
        fs.create(name.as_bytes()).expect("create must succeed")
    }

    fn contents(fs: &mut FileSystem, handle: Handle) -> Vec<u8> {
        let mut buf = vec![0u8; 4096];
        let copied = fs.read(handle, 0, &mut buf).expect("read must succeed");
        buf.truncate(copied);
        buf
    }

    fn name_of_len(length: usize) -> String {
        (0..length).map(|_| 'k').collect()
    }

    #[test]
    fn limits_are_pinned() {
        assert_eq!(MAX_NAME_BYTES, 255);
        assert_eq!(MAX_PATH_BYTES, 4096);
        assert_eq!(MAX_DEPTH, 512);
        assert_eq!(MAX_OPEN_FILES, 64);
    }

    #[test]
    fn lookup_of_missing_name_is_not_found() {
        let mut fs = FileSystem::new();
        assert_eq!(fs.lookup(b"/ghost.txt"), Err(FsError::NotFound));
    }

    #[test]
    fn lookup_of_root_is_directory() {
        let mut fs = FileSystem::new();
        assert_eq!(
            fs.lookup(b"/").unwrap(),
            Stat {
                kind: Kind::Directory,
                size: 0
            }
        );
    }

    #[test]
    fn lookup_of_regular_name_is_regular_with_exact_size() {
        let mut fs = FileSystem::new();
        let handle = create(&mut fs, "/a.txt");
        fs.write(handle, 0, b"hello").unwrap();
        assert_eq!(
            fs.lookup(b"/a.txt").unwrap(),
            Stat {
                kind: Kind::Regular,
                size: 5
            }
        );
    }

    #[test]
    fn create_then_open_reuse_the_same_stored_bytes() {
        let mut fs = FileSystem::new();
        let created = create(&mut fs, "/a.txt");
        fs.write(created, 0, b"abc").unwrap();
        fs.close(created).unwrap();
        let reopened = fs.open(b"/a.txt").unwrap();
        assert_eq!(contents(&mut fs, reopened), b"abc".to_vec());
    }

    #[test]
    fn create_existing_name_is_already_exists() {
        let mut fs = FileSystem::new();
        create(&mut fs, "/a.txt");
        assert_eq!(fs.create(b"/a.txt"), Err(FsError::AlreadyExists));
    }

    #[test]
    fn open_missing_name_is_not_found() {
        let mut fs = FileSystem::new();
        assert_eq!(fs.open(b"/ghost.txt"), Err(FsError::NotFound));
    }

    #[test]
    fn create_at_root_is_already_exists() {
        let mut fs = FileSystem::new();
        assert_eq!(fs.create(b"/"), Err(FsError::AlreadyExists));
    }

    #[test]
    fn open_at_root_is_a_directory() {
        let mut fs = FileSystem::new();
        assert_eq!(fs.open(b"/"), Err(FsError::IsADirectory));
    }

    #[test]
    fn malformed_paths_are_invalid_regardless_of_depth() {
        let mut fs = FileSystem::new();
        assert_eq!(fs.open(b"a.txt"), Err(FsError::InvalidPath));
        assert_eq!(fs.open(b""), Err(FsError::InvalidPath));
        assert_eq!(fs.open(b"//a"), Err(FsError::InvalidPath));
        assert_eq!(fs.open(b"/a//"), Err(FsError::InvalidPath));
        assert_eq!(fs.create(b"a.txt"), Err(FsError::InvalidPath));
    }

    #[test]
    fn nested_paths_resolve_and_report_missing_parents_as_not_found() {
        let mut fs = FileSystem::new();
        assert_eq!(fs.lookup(b"/a/b"), Err(FsError::NotFound));
        assert_eq!(fs.open(b"/a/b"), Err(FsError::NotFound));
        assert_eq!(fs.create(b"/a/b"), Err(FsError::NotFound));
        assert_eq!(fs.remove(b"/a/b"), Err(FsError::NotFound));
        assert_eq!(fs.readdir(b"/a/b"), Err(FsError::NotFound));
    }

    #[test]
    fn nested_path_under_regular_file_parent_is_not_a_directory() {
        let mut fs = FileSystem::new();
        create(&mut fs, "/a");
        assert_eq!(fs.lookup(b"/a/b"), Err(FsError::NotADirectory));
        assert_eq!(fs.open(b"/a/b"), Err(FsError::NotADirectory));
        assert_eq!(fs.create(b"/a/b"), Err(FsError::NotADirectory));
        assert_eq!(fs.readdir(b"/a/b"), Err(FsError::NotADirectory));
    }

    #[test]
    fn deep_path_at_limit_is_well_formed_and_past_it_is_depth_exceeded() {
        let mut fs = FileSystem::new();
        let at_limit: String = (0..MAX_DEPTH).map(|_| "/k").collect();
        assert_eq!(fs.lookup(at_limit.as_bytes()), Err(FsError::NotFound));
        let past_limit: String = (0..MAX_DEPTH + 1).map(|_| "/k").collect();
        assert_eq!(
            fs.lookup(past_limit.as_bytes()),
            Err(FsError::DepthExceeded)
        );
    }

    #[test]
    fn trailing_slash_names_the_same_regular_file() {
        let mut fs = FileSystem::new();
        create(&mut fs, "/a.txt");
        assert_eq!(
            fs.lookup(b"/a.txt/").unwrap(),
            Stat {
                kind: Kind::Regular,
                size: 0
            }
        );
    }

    #[test]
    fn name_at_limit_is_accepted_and_past_it_is_name_too_long() {
        let mut fs = FileSystem::new();
        let at_limit = format!("/{}", name_of_len(255));
        create(&mut fs, &at_limit);
        let past_limit = format!("/{}", name_of_len(256));
        assert_eq!(fs.open(past_limit.as_bytes()), Err(FsError::NameTooLong));
    }

    #[test]
    fn path_past_limit_is_path_too_long() {
        let mut fs = FileSystem::new();
        let long = format!("/{}", name_of_len(4096));
        assert_eq!(fs.open(long.as_bytes()), Err(FsError::PathTooLong));
    }

    #[test]
    fn nul_byte_in_name_is_invalid_path() {
        let mut fs = FileSystem::new();
        assert_eq!(fs.create(b"/bad\0name"), Err(FsError::InvalidPath));
    }

    #[test]
    fn read_at_or_past_end_is_empty() {
        let mut fs = FileSystem::new();
        let handle = create(&mut fs, "/a.txt");
        fs.write(handle, 0, b"abc").unwrap();
        let mut buf = [0u8; 10];
        assert_eq!(fs.read(handle, 3, &mut buf).unwrap(), 0);
        assert_eq!(fs.read(handle, 999, &mut buf).unwrap(), 0);
    }

    #[test]
    fn read_clamps_across_end() {
        let mut fs = FileSystem::new();
        let handle = create(&mut fs, "/a.txt");
        fs.write(handle, 0, b"abcde").unwrap();
        let mut buf = [0u8; 100];
        assert_eq!(fs.read(handle, 3, &mut buf).unwrap(), 2);
        assert_eq!(&buf[..2], b"de");
    }

    #[test]
    fn write_appends_at_len_and_beyond_zero_fills() {
        let mut fs = FileSystem::new();
        let handle = create(&mut fs, "/a.txt");
        fs.write(handle, 0, b"ab").unwrap();
        assert_eq!(fs.write(handle, 4, b"cd").unwrap(), 2);
        assert_eq!(contents(&mut fs, handle), vec![b'a', b'b', 0, 0, b'c', b'd']);
    }

    #[test]
    fn empty_write_is_noop_and_never_resizes() {
        let mut fs = FileSystem::new();
        let handle = create(&mut fs, "/a.txt");
        fs.write(handle, 0, b"abc").unwrap();
        assert_eq!(fs.write(handle, 100, &[]).unwrap(), 0);
        assert_eq!(contents(&mut fs, handle), b"abc".to_vec());
    }

    #[test]
    fn close_frees_slot_and_second_close_is_bad_handle() {
        let mut fs = FileSystem::new();
        let handle = create(&mut fs, "/a.txt");
        assert_eq!(fs.close(handle), Ok(()));
        assert_eq!(fs.close(handle), Err(FsError::BadHandle));
    }

    #[test]
    fn stale_handle_after_close_and_reopen_is_bad_handle() {
        let mut fs = FileSystem::new();
        let first = create(&mut fs, "/a.txt");
        fs.close(first).unwrap();
        let second = fs.open(b"/a.txt").unwrap();
        assert_ne!(first, second);
        assert_eq!(fs.read(second, 0, &mut [0u8; 4]).unwrap(), 0);
        fs.write(first, 0, b"x")
            .expect_err("stale handle must be rejected");
    }

    #[test]
    fn remove_then_lookup_is_not_found() {
        let mut fs = FileSystem::new();
        create(&mut fs, "/a.txt");
        assert_eq!(fs.remove(b"/a.txt"), Ok(()));
        assert_eq!(fs.lookup(b"/a.txt"), Err(FsError::NotFound));
    }

    #[test]
    fn remove_missing_is_not_found() {
        let mut fs = FileSystem::new();
        assert_eq!(fs.remove(b"/ghost.txt"), Err(FsError::NotFound));
    }

    #[test]
    fn remove_at_root_is_invalid_path() {
        let mut fs = FileSystem::new();
        assert_eq!(fs.remove(b"/"), Err(FsError::InvalidPath));
    }

    #[test]
    fn open_handle_to_removed_file_is_bad_handle_on_next_use() {
        let mut fs = FileSystem::new();
        let handle = create(&mut fs, "/a.txt");
        fs.write(handle, 0, b"abc").unwrap();
        fs.remove(b"/a.txt").unwrap();
        assert_eq!(
            fs.read(handle, 0, &mut [0u8; 4]),
            Err(FsError::BadHandle)
        );
        assert_eq!(fs.write(handle, 0, b"x"), Err(FsError::BadHandle));
    }

    #[test]
    fn handle_to_removed_file_still_closes_cleanly() {
        let mut fs = FileSystem::new();
        let handle = create(&mut fs, "/a.txt");
        fs.remove(b"/a.txt").unwrap();
        assert_eq!(fs.close(handle), Ok(()));
    }

    #[test]
    fn recreate_during_open_isolates_the_two_generations() {
        let mut fs = FileSystem::new();
        let first = create(&mut fs, "/a.txt");
        fs.write(first, 0, b"one").unwrap();
        fs.remove(b"/a.txt").unwrap();
        let second = create(&mut fs, "/a.txt");
        fs.write(second, 0, b"two").unwrap();
        assert_eq!(contents(&mut fs, second), b"two".to_vec());
        assert_eq!(fs.read(first, 0, &mut [0u8; 4]), Err(FsError::BadHandle));
    }

    #[test]
    fn two_handles_on_one_name_share_content() {
        let mut fs = FileSystem::new();
        let writer = create(&mut fs, "/a.txt");
        let reader = fs.open(b"/a.txt").unwrap();
        fs.write(writer, 0, b"shared").unwrap();
        assert_eq!(contents(&mut fs, reader), b"shared".to_vec());
    }

    #[test]
    fn readdir_root_is_byte_sorted() {
        let mut fs = FileSystem::new();
        create(&mut fs, "/notes.txt");
        create(&mut fs, "/blob.bin");
        create(&mut fs, "/alpha");
        let items = fs.readdir(b"/").unwrap();
        let names: Vec<&[u8]> = items.iter().map(|item| item.name.as_slice()).collect();
        assert_eq!(names, vec![b"alpha".as_slice(), b"blob.bin", b"notes.txt"]);
        assert!(items.iter().all(|item| item.kind == Kind::Regular));
    }

    #[test]
    fn readdir_of_regular_name_is_not_a_directory() {
        let mut fs = FileSystem::new();
        create(&mut fs, "/a.txt");
        assert_eq!(fs.readdir(b"/a.txt"), Err(FsError::NotADirectory));
    }

    #[test]
    fn readdir_empty_root_is_empty() {
        let mut fs = FileSystem::new();
        assert_eq!(fs.readdir(b"/").unwrap(), Vec::new());
    }

    #[test]
    fn open_beyond_slot_cap_is_too_many_open() {
        let mut fs = FileSystem::new();
        let target = create(&mut fs, "/overflow");
        fs.close(target).unwrap();
        let mut held = Vec::new();
        for index in 0..MAX_OPEN_FILES {
            held.push(create(&mut fs, &format!("/s{index}")));
        }
        assert_eq!(fs.open(b"/overflow"), Err(FsError::TooManyOpen));
        assert_eq!(fs.close(held[0]), Ok(()));
        let revived = fs.open(b"/overflow").unwrap();
        assert!(fs.close(revived).is_ok());
    }

    #[test]
    fn create_beyond_slot_cap_is_too_many_open_without_leaving_a_file() {
        let mut fs = FileSystem::new();
        for index in 0..MAX_OPEN_FILES {
            create(&mut fs, &format!("/s{index}"));
        }
        assert_eq!(fs.create(b"/late"), Err(FsError::TooManyOpen));
        assert_eq!(fs.lookup(b"/late"), Err(FsError::NotFound));
    }

    #[test]
    fn stat_tracks_writes_and_holes() {
        let mut fs = FileSystem::new();
        let handle = create(&mut fs, "/a.txt");
        assert_eq!(fs.stat(handle).unwrap().size, 0);
        fs.write(handle, 0, b"ab").unwrap();
        fs.write(handle, 4096, b"c").unwrap();
        assert_eq!(fs.stat(handle).unwrap().size, 4097);
    }

    #[test]
    fn sync_succeeds_on_a_plain_memory_volume() {
        let mut fs = FileSystem::new();
        assert_eq!(fs.sync(), Ok(()));
    }
}
