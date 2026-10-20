use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;
use core::cmp::min;
use system::rom::{RomError, RomImage};
use vfs_protocol::wire;
use vfs_protocol::wire::VfsError;

pub struct OpenEntry {
    pub owner: u32,
    pub mode: u8,
    pub name: String,
}

pub enum FileStore {
    Ram(Vec<u8>),
    Rom { offset: usize, len: usize },
}

#[derive(Debug)]
pub enum RomMountError {
    Format(RomError),
    InvalidName(String),
    Duplicate(String),
    AlreadyAttached,
}

pub struct RamFs {
    files: BTreeMap<String, FileStore>,
    opens: BTreeMap<u32, OpenEntry>,
    next_handle: u32,
    rom_blob: Vec<u8>,
    rom_attached: bool,
}

impl RamFs {
    pub fn new() -> RamFs {
        RamFs {
            files: BTreeMap::new(),
            opens: BTreeMap::new(),
            next_handle: 1,
            rom_blob: Vec::new(),
            rom_attached: false,
        }
    }

    pub fn attach_rom(&mut self, image: Vec<u8>) -> Result<(), RomMountError> {
        if self.rom_attached {
            return Err(RomMountError::AlreadyAttached);
        }
        let parsed = RomImage::parse(&image).map_err(RomMountError::Format)?;
        let mut staged: Vec<(&str, usize, usize)> = Vec::new();
        for entry in parsed.files() {
            if !wire::valid_name(entry.name) {
                return Err(RomMountError::InvalidName(String::from(entry.name)));
            }
            if self.files.contains_key(entry.name) {
                return Err(RomMountError::Duplicate(String::from(entry.name)));
            }
            staged.push((entry.name, entry.offset, entry.len));
        }
        for (name, offset, len) in staged {
            self.files
                .insert(String::from(name), FileStore::Rom { offset, len });
        }
        self.rom_blob = image;
        self.rom_attached = true;
        Ok(())
    }

    pub fn open(&mut self, owner: u32, mode: u8, name: &str) -> Result<u32, VfsError> {
        if !wire::valid_name(name) {
            return Err(VfsError::InvalidPath);
        }
        if self.files.contains_key(name) {
            if mode & wire::MODE_CREATE != 0 {
                return Err(VfsError::AlreadyExists);
            }
        } else {
            if mode & wire::MODE_CREATE == 0 {
                return Err(VfsError::NotFound);
            }
            self.files
                .insert(String::from(name), FileStore::Ram(Vec::new()));
        }
        let handle = self.next_handle;
        self.next_handle += 1;
        self.opens.insert(
            handle,
            OpenEntry {
                owner,
                mode,
                name: String::from(name),
            },
        );
        Ok(handle)
    }

    pub fn read(&self, owner: u32, handle: u32, offset: u64, count: usize) -> Result<Vec<u8>, VfsError> {
        let Some(entry) = self.opens.get(&handle) else {
            return Err(VfsError::BadHandle);
        };
        if entry.owner != owner {
            return Err(VfsError::BadHandle);
        }
        let Some(store) = self.files.get(entry.name.as_str()) else {
            return Err(VfsError::NotFound);
        };
        let data = match store {
            FileStore::Ram(data) => data.as_slice(),
            FileStore::Rom { offset: window, len } => {
                &self.rom_blob[*window..*window + *len]
            }
        };
        let start = min(offset, data.len() as u64) as usize;
        let length = min(count, data.len() - start);
        Ok(data[start..start + length].to_vec())
    }

    pub fn write(&mut self, owner: u32, handle: u32, offset: u64, payload: &[u8]) -> Result<usize, VfsError> {
        let Some(entry) = self.opens.get(&handle) else {
            return Err(VfsError::BadHandle);
        };
        if entry.owner != owner {
            return Err(VfsError::BadHandle);
        }
        let mode = entry.mode;
        let name = String::clone(&entry.name);
        let file = match self.files.get_mut(&name) {
            None => return Err(VfsError::NotFound),
            Some(FileStore::Rom { .. }) => return Err(VfsError::NotPermitted),
            Some(FileStore::Ram(file)) => file,
        };
        if payload.is_empty() {
            return Ok(0);
        }
        if mode & wire::MODE_WRITE == 0 {
            return Err(VfsError::NotPermitted);
        }
        let start = offset as usize;
        let end = start.saturating_add(payload.len());
        if file.len() < end {
            file.resize(end, 0);
        }
        file[start..end].copy_from_slice(payload);
        Ok(payload.len())
    }

    pub fn close(&mut self, owner: u32, handle: u32) -> Result<(), VfsError> {
        match self.opens.get(&handle) {
            Some(entry) if entry.owner == owner => {}
            _ => return Err(VfsError::BadHandle),
        }
        self.opens.remove(&handle);
        Ok(())
    }

    pub fn delete(&mut self, name: &str) -> Result<(), VfsError> {
        if !wire::valid_name(name) {
            return Err(VfsError::InvalidPath);
        }
        match self.files.get(name) {
            None => return Err(VfsError::NotFound),
            Some(FileStore::Rom { .. }) => return Err(VfsError::NotPermitted),
            Some(FileStore::Ram(_)) => {}
        }
        self.files.remove(name);
        Ok(())
    }

    pub fn list(&self) -> Result<Vec<u8>, VfsError> {
        let mut blob = Vec::new();
        for name in self.files.keys() {
            if !blob.is_empty() {
                blob.push(b'\n');
            }
            blob.extend_from_slice(name.as_bytes());
        }
        if blob.len() > wire::RPC_MAX_DATA_BYTES {
            return Err(VfsError::ListTooLarge);
        }
        Ok(blob)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::format;

    const OWNER: u32 = 0x0010_0001;
    const INTRUDER: u32 = 0x0020_0002;

    fn create(fs: &mut RamFs, owner: u32, name: &str) -> u32 {
        fs.open(owner, wire::MODE_CREATE | wire::MODE_WRITE, name)
            .expect("create must succeed")
    }

    fn contents(fs: &RamFs, owner: u32, handle: u32) -> Vec<u8> {
        fs.read(owner, handle, 0, wire::RPC_MAX_DATA_BYTES)
            .expect("read must succeed")
    }

    fn name_of_len(length: usize) -> String {
        (0..length).map(|_| 'k').collect()
    }

    #[test]
    fn open_with_create_creates_empty_file_and_first_handle_is_one() {
        let mut fs = RamFs::new();
        let handle = create(&mut fs, OWNER, "a.txt");
        assert_eq!(handle, 1);
        assert_eq!(contents(&fs, OWNER, handle), Vec::new());
    }

    #[test]
    fn open_existing_without_create_reuses_existing_content() {
        let mut fs = RamFs::new();
        let first = create(&mut fs, OWNER, "a.txt");
        fs.write(OWNER, first, 0, b"abc").unwrap();
        fs.close(OWNER, first).unwrap();
        let second = fs.open(OWNER, 0, "a.txt").expect("plain open must succeed");
        assert_eq!(second, 2);
        assert_eq!(contents(&fs, OWNER, second), b"abc".to_vec());
    }

    #[test]
    fn open_existing_with_create_is_already_exists() {
        let mut fs = RamFs::new();
        create(&mut fs, OWNER, "a.txt");
        assert_eq!(
            fs.open(OWNER, wire::MODE_CREATE, "a.txt"),
            Err(VfsError::AlreadyExists)
        );
    }

    #[test]
    fn open_missing_without_create_is_not_found() {
        let mut fs = RamFs::new();
        assert_eq!(fs.open(OWNER, 0, "ghost.txt"), Err(VfsError::NotFound));
    }

    #[test]
    fn handles_increase_monotonically() {
        let mut fs = RamFs::new();
        assert_eq!(create(&mut fs, OWNER, "a"), 1);
        assert_eq!(create(&mut fs, OWNER, "b"), 2);
        assert_eq!(create(&mut fs, OWNER, "c"), 3);
    }

    #[test]
    fn valid_name_accepts_one_to_fifty_seven_bytes() {
        assert!(wire::valid_name("a"));
        assert!(wire::valid_name(&name_of_len(57)));
    }

    #[test]
    fn valid_name_rejects_empty_oversize_and_forbidden_bytes() {
        assert!(!wire::valid_name(""));
        assert!(!wire::valid_name(&name_of_len(58)));
        assert!(!wire::valid_name("bad\nname"));
        assert!(!wire::valid_name("bad\0name"));
    }

    #[test]
    fn open_invalid_name_is_invalid_path() {
        let mut fs = RamFs::new();
        assert_eq!(
            fs.open(OWNER, wire::MODE_CREATE, ""),
            Err(VfsError::InvalidPath)
        );
        assert_eq!(
            fs.open(OWNER, wire::MODE_CREATE, &name_of_len(58)),
            Err(VfsError::InvalidPath)
        );
        assert_eq!(
            fs.open(OWNER, wire::MODE_CREATE, "bad\nname"),
            Err(VfsError::InvalidPath)
        );
    }

    #[test]
    fn delete_invalid_name_is_invalid_path() {
        let mut fs = RamFs::new();
        assert_eq!(fs.delete("bad\nname"), Err(VfsError::InvalidPath));
        assert_eq!(fs.delete(""), Err(VfsError::InvalidPath));
    }

    #[test]
    fn read_at_end_is_empty() {
        let mut fs = RamFs::new();
        let handle = create(&mut fs, OWNER, "a.txt");
        fs.write(OWNER, handle, 0, b"abc").unwrap();
        assert_eq!(fs.read(OWNER, handle, 3, 10).unwrap(), Vec::new());
    }

    #[test]
    fn read_past_end_is_empty() {
        let mut fs = RamFs::new();
        let handle = create(&mut fs, OWNER, "a.txt");
        fs.write(OWNER, handle, 0, b"abc").unwrap();
        assert_eq!(fs.read(OWNER, handle, 999, 10).unwrap(), Vec::new());
    }

    #[test]
    fn read_clamps_across_end() {
        let mut fs = RamFs::new();
        let handle = create(&mut fs, OWNER, "a.txt");
        fs.write(OWNER, handle, 0, b"abcde").unwrap();
        assert_eq!(fs.read(OWNER, handle, 3, 100).unwrap(), b"de".to_vec());
    }

    #[test]
    fn unknown_handle_is_bad_handle_for_every_operation() {
        let mut fs = RamFs::new();
        assert_eq!(fs.read(OWNER, 42, 0, 10), Err(VfsError::BadHandle));
        assert_eq!(fs.write(OWNER, 42, 0, b"x"), Err(VfsError::BadHandle));
        assert_eq!(fs.close(OWNER, 42), Err(VfsError::BadHandle));
    }

    #[test]
    fn cross_owner_handle_use_is_bad_handle() {
        let mut fs = RamFs::new();
        let handle = create(&mut fs, OWNER, "a.txt");
        fs.write(OWNER, handle, 0, b"secret").unwrap();
        assert_eq!(fs.read(INTRUDER, handle, 0, 10), Err(VfsError::BadHandle));
        assert_eq!(fs.write(INTRUDER, handle, 0, b"x"), Err(VfsError::BadHandle));
        assert_eq!(fs.close(INTRUDER, handle), Err(VfsError::BadHandle));
        assert_eq!(contents(&fs, OWNER, handle), b"secret".to_vec());
        assert_eq!(fs.close(OWNER, handle), Ok(()));
    }

    #[test]
    fn write_without_write_mode_is_not_permitted() {
        let mut fs = RamFs::new();
        let created = create(&mut fs, OWNER, "a.txt");
        fs.write(OWNER, created, 0, b"abc").unwrap();
        fs.close(OWNER, created).unwrap();
        let readonly = fs.open(OWNER, 0, "a.txt").unwrap();
        assert_eq!(
            fs.write(OWNER, readonly, 0, b"x"),
            Err(VfsError::NotPermitted)
        );
        assert_eq!(contents(&fs, OWNER, readonly), b"abc".to_vec());
    }

    #[test]
    fn read_is_permitted_on_mode_zero_handle() {
        let mut fs = RamFs::new();
        let created = create(&mut fs, OWNER, "a.txt");
        fs.write(OWNER, created, 0, b"hello").unwrap();
        fs.close(OWNER, created).unwrap();
        let readonly = fs.open(OWNER, 0, "a.txt").unwrap();
        assert_eq!(contents(&fs, OWNER, readonly), b"hello".to_vec());
    }

    #[test]
    fn write_at_zero_into_empty_file() {
        let mut fs = RamFs::new();
        let handle = create(&mut fs, OWNER, "a.txt");
        assert_eq!(fs.write(OWNER, handle, 0, b"hi"), Ok(2));
        assert_eq!(contents(&fs, OWNER, handle), b"hi".to_vec());
    }

    #[test]
    fn write_appending_at_len() {
        let mut fs = RamFs::new();
        let handle = create(&mut fs, OWNER, "a.txt");
        fs.write(OWNER, handle, 0, b"ab").unwrap();
        assert_eq!(fs.write(OWNER, handle, 2, b"cd"), Ok(2));
        assert_eq!(contents(&fs, OWNER, handle), b"abcd".to_vec());
    }

    #[test]
    fn write_beyond_len_zero_fills_hole_byte_exactly() {
        let mut fs = RamFs::new();
        let handle = create(&mut fs, OWNER, "a.txt");
        fs.write(OWNER, handle, 0, b"ab").unwrap();
        fs.write(OWNER, handle, 4, b"cd").unwrap();
        assert_eq!(
            contents(&fs, OWNER, handle),
            vec![b'a', b'b', 0, 0, b'c', b'd']
        );
    }

    #[test]
    fn empty_write_is_noop_and_never_resizes() {
        let mut fs = RamFs::new();
        let handle = create(&mut fs, OWNER, "a.txt");
        fs.write(OWNER, handle, 0, b"abc").unwrap();
        assert_eq!(fs.write(OWNER, handle, 100, &[]), Ok(0));
        assert_eq!(contents(&fs, OWNER, handle), b"abc".to_vec());
    }

    #[test]
    fn close_frees_handle_and_second_close_is_bad_handle() {
        let mut fs = RamFs::new();
        let handle = create(&mut fs, OWNER, "a.txt");
        assert_eq!(fs.close(OWNER, handle), Ok(()));
        assert_eq!(fs.close(OWNER, handle), Err(VfsError::BadHandle));
    }

    #[test]
    fn delete_then_read_via_still_open_handle_is_not_found() {
        let mut fs = RamFs::new();
        let handle = create(&mut fs, OWNER, "a.txt");
        fs.write(OWNER, handle, 0, b"abc").unwrap();
        assert_eq!(fs.delete("a.txt"), Ok(()));
        assert_eq!(fs.read(OWNER, handle, 0, 10), Err(VfsError::NotFound));
        assert_eq!(fs.write(OWNER, handle, 0, b"x"), Err(VfsError::NotFound));
    }

    #[test]
    fn handle_to_deleted_file_still_closes_cleanly() {
        let mut fs = RamFs::new();
        let handle = create(&mut fs, OWNER, "a.txt");
        fs.delete("a.txt").unwrap();
        assert_eq!(fs.close(OWNER, handle), Ok(()));
    }

    #[test]
    fn delete_missing_is_not_found() {
        let mut fs = RamFs::new();
        assert_eq!(fs.delete("ghost.txt"), Err(VfsError::NotFound));
    }

    #[test]
    fn list_returns_btree_ordered_newline_joined_names() {
        let mut fs = RamFs::new();
        create(&mut fs, OWNER, "notes.txt");
        create(&mut fs, OWNER, "blob.bin");
        assert_eq!(fs.list().unwrap(), b"blob.bin\nnotes.txt".to_vec());
    }

    #[test]
    fn list_empty_store_is_empty() {
        let fs = RamFs::new();
        assert_eq!(fs.list().unwrap(), Vec::new());
    }

    #[test]
    fn two_handles_on_one_file_see_shared_content() {
        let mut fs = RamFs::new();
        let writer = create(&mut fs, OWNER, "a.txt");
        let reader = fs.open(OWNER, 0, "a.txt").unwrap();
        fs.write(OWNER, writer, 0, b"shared").unwrap();
        assert_eq!(contents(&fs, OWNER, reader), b"shared".to_vec());
        assert_eq!(contents(&fs, OWNER, writer), b"shared".to_vec());
        fs.write(OWNER, reader, 0, b"x")
            .expect_err("mode zero must reject write");
    }

    #[test]
    fn list_of_joined_names_past_limit_is_list_too_large() {
        let mut fs = RamFs::new();
        for index in 0..1200u32 {
            let name = format!("{index:0>4}{}", name_of_len(52));
            create(&mut fs, OWNER, &name);
        }
        assert_eq!(fs.list(), Err(VfsError::ListTooLarge));
    }

    fn fabricated_rom() -> Vec<u8> {
        system::rom::encode(&[
            ("beta.bin", b"beta-payload".as_slice()),
            ("alpha.bin", b"alpha".as_slice()),
        ])
    }

    #[test]
    fn attach_rom_lists_rom_names_sorted_with_ram_names() {
        let mut fs = RamFs::new();
        fs.attach_rom(fabricated_rom()).unwrap();
        create(&mut fs, OWNER, "mid.txt");
        assert_eq!(fs.list().unwrap(), b"alpha.bin\nbeta.bin\nmid.txt".to_vec());
    }

    #[test]
    fn rom_file_reads_exact_bytes() {
        let mut fs = RamFs::new();
        fs.attach_rom(fabricated_rom()).unwrap();
        let handle = fs.open(OWNER, 0, "alpha.bin").unwrap();
        assert_eq!(contents(&fs, OWNER, handle), b"alpha".to_vec());
        assert_eq!(fs.read(OWNER, handle, 2, 2).unwrap(), b"ph".to_vec());
    }

    #[test]
    fn rom_read_clamps_across_window_end_and_eof() {
        let mut fs = RamFs::new();
        fs.attach_rom(fabricated_rom()).unwrap();
        let handle = fs.open(OWNER, 0, "beta.bin").unwrap();
        assert_eq!(fs.read(OWNER, handle, 5, 100).unwrap(), b"payload".to_vec());
        assert_eq!(fs.read(OWNER, handle, 12, 10).unwrap(), Vec::new());
        assert_eq!(fs.read(OWNER, handle, 999, 10).unwrap(), Vec::new());
    }

    #[test]
    fn open_rom_with_create_is_already_exists() {
        let mut fs = RamFs::new();
        fs.attach_rom(fabricated_rom()).unwrap();
        assert_eq!(
            fs.open(OWNER, wire::MODE_CREATE, "alpha.bin"),
            Err(VfsError::AlreadyExists)
        );
    }

    #[test]
    fn write_to_rom_file_is_not_permitted_even_when_empty() {
        let mut fs = RamFs::new();
        fs.attach_rom(fabricated_rom()).unwrap();
        let handle = fs.open(OWNER, wire::MODE_WRITE, "alpha.bin").unwrap();
        assert_eq!(fs.write(OWNER, handle, 0, b"x"), Err(VfsError::NotPermitted));
        assert_eq!(fs.write(OWNER, handle, 0, &[]), Err(VfsError::NotPermitted));
        assert_eq!(fs.write(OWNER, handle, 99, &[]), Err(VfsError::NotPermitted));
        assert_eq!(contents(&fs, OWNER, handle), b"alpha".to_vec());
    }

    #[test]
    fn delete_rom_file_is_not_permitted() {
        let mut fs = RamFs::new();
        fs.attach_rom(fabricated_rom()).unwrap();
        assert_eq!(fs.delete("alpha.bin"), Err(VfsError::NotPermitted));
        assert_eq!(fs.list().unwrap(), b"alpha.bin\nbeta.bin".to_vec());
    }

    #[test]
    fn second_attach_rom_is_already_attached() {
        let mut fs = RamFs::new();
        fs.attach_rom(fabricated_rom()).unwrap();
        assert!(matches!(
            fs.attach_rom(fabricated_rom()),
            Err(RomMountError::AlreadyAttached)
        ));
    }

    #[test]
    fn attach_rom_rejects_format_invalid_image() {
        let mut fs = RamFs::new();
        assert!(matches!(
            fs.attach_rom(b"not a rom image at all".to_vec()),
            Err(RomMountError::Format(RomError::BadMagic))
        ));
        assert_eq!(fs.list().unwrap(), Vec::new());
    }

    #[test]
    fn attach_rom_rejects_name_invalid_for_vfs() {
        let mut fs = RamFs::new();
        let image = system::rom::encode(&[("bad\nname", b"x".as_slice())]);
        assert!(matches!(
            fs.attach_rom(image),
            Err(RomMountError::InvalidName(ref name)) if name == "bad\nname"
        ));
        assert_eq!(fs.list().unwrap(), Vec::new());
    }

    #[test]
    fn attach_rom_rejects_collision_and_stores_nothing() {
        let mut fs = RamFs::new();
        let handle = create(&mut fs, OWNER, "alpha.bin");
        assert!(matches!(
            fs.attach_rom(fabricated_rom()),
            Err(RomMountError::Duplicate(ref name)) if name == "alpha.bin"
        ));
        assert_eq!(fs.list().unwrap(), b"alpha.bin".to_vec());
        fs.write(OWNER, handle, 0, b"still writable")
            .expect("collision rejection must not convert RAM to ROM");
        assert_eq!(contents(&fs, OWNER, handle), b"still writable".to_vec());
    }
}
