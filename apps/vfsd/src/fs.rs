use alloc::collections::{BTreeMap, BTreeSet};
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use rxfs::device::MemoryDevice;
use rxfs::error::FsError;
use rxfs::spec::DEFAULT_BYTES_PER_INODE;
use rxfs::superblock::UUID_BYTES;
use rxfs::volume::{FileSystem, FormatParams, Handle};
use system::rom::{RomError, RomImage};
use vfs_protocol::wire;
use vfs_protocol::wire::VfsError;

const VOLUME_BLOCKS: u64 = 2048;
const VOLUME_UUID: [u8; UUID_BYTES] = *b"rosx-ram-vol-001";
const VOLUME_NAME: &[u8] = b"rosx-ram";

struct OpenEntry {
    owner: u32,
    mode: u8,
    name: String,
    rxfs_handle: Handle,
}

#[derive(Debug)]
pub enum RomMountError {
    Format(RomError),
    InvalidName(String),
    Duplicate(String),
    AlreadyAttached,
    Engine(FsError),
}

pub struct VfsStore {
    fs: FileSystem<MemoryDevice>,
    opens: BTreeMap<u32, OpenEntry>,
    next_handle: u32,
    rom_names: BTreeSet<String>,
    rom_attached: bool,
}

fn volume_path(name: &str) -> Vec<u8> {
    let mut path = Vec::with_capacity(name.len() + 1);
    path.push(b'/');
    path.extend_from_slice(name.as_bytes());
    path
}

fn dot_entry(name: &[u8]) -> bool {
    name == b".".as_slice() || name == b"..".as_slice()
}

fn map_fs_error(error: FsError) -> VfsError {
    match error {
        FsError::NotFound => VfsError::NotFound,
        FsError::AlreadyExists => VfsError::AlreadyExists,
        FsError::InvalidPath | FsError::PathTooLong | FsError::NameTooLong | FsError::InvalidName => {
            VfsError::InvalidPath
        }
        FsError::BadHandle => VfsError::BadHandle,
        _ => VfsError::ServerResource,
    }
}

impl VfsStore {
    pub fn new() -> VfsStore {
        let params = FormatParams {
            bytes_per_inode: DEFAULT_BYTES_PER_INODE,
            uuid: VOLUME_UUID,
            volume_name: VOLUME_NAME,
            created_at: 0,
        };
        let fs = FileSystem::format(MemoryDevice::new(VOLUME_BLOCKS), &params)
            .expect("fresh memory volume must format");
        VfsStore {
            fs,
            opens: BTreeMap::new(),
            next_handle: 1,
            rom_names: BTreeSet::new(),
            rom_attached: false,
        }
    }

    pub fn attach_rom(&mut self, image: &[u8]) -> Result<(), RomMountError> {
        if self.rom_attached {
            return Err(RomMountError::AlreadyAttached);
        }
        let parsed = RomImage::parse(image).map_err(RomMountError::Format)?;
        let mut staged: Vec<(&str, usize, usize)> = Vec::new();
        for entry in parsed.files() {
            if !wire::valid_name(entry.name) {
                return Err(RomMountError::InvalidName(String::from(entry.name)));
            }
            if self.fs.lookup(&volume_path(entry.name)).is_ok() {
                return Err(RomMountError::Duplicate(String::from(entry.name)));
            }
            staged.push((entry.name, entry.offset, entry.len));
        }
        for (name, offset, len) in staged {
            let handle = self
                .fs
                .create(&volume_path(name))
                .map_err(RomMountError::Engine)?;
            if len > 0 {
                self.fs
                    .write(handle, 0, &image[offset..offset + len])
                    .map_err(RomMountError::Engine)?;
            }
            self.fs.close(handle).map_err(RomMountError::Engine)?;
            self.rom_names.insert(String::from(name));
        }
        self.fs.sync().map_err(RomMountError::Engine)?;
        self.rom_attached = true;
        Ok(())
    }

    pub fn open(&mut self, owner: u32, mode: u8, name: &str) -> Result<u32, VfsError> {
        if !wire::valid_name(name) {
            return Err(VfsError::InvalidPath);
        }
        let path = volume_path(name);
        let rxfs_handle = match self.fs.lookup(&path) {
            Ok(_) if mode & wire::MODE_CREATE != 0 => return Err(VfsError::AlreadyExists),
            Ok(_) => self.fs.open(&path).map_err(map_fs_error)?,
            Err(FsError::NotFound) if mode & wire::MODE_CREATE != 0 => {
                let handle = self.fs.create(&path).map_err(map_fs_error)?;
                self.fs.sync().map_err(map_fs_error)?;
                handle
            }
            Err(FsError::NotFound) => return Err(VfsError::NotFound),
            Err(error) => return Err(map_fs_error(error)),
        };
        let handle = self.next_handle;
        self.next_handle += 1;
        self.opens.insert(
            handle,
            OpenEntry {
                owner,
                mode,
                name: String::from(name),
                rxfs_handle,
            },
        );
        Ok(handle)
    }

    pub fn read(
        &mut self,
        owner: u32,
        handle: u32,
        offset: u64,
        count: usize,
    ) -> Result<Vec<u8>, VfsError> {
        let rxfs_handle = self.live_handle(owner, handle)?;
        let mut buf = vec![0u8; count];
        let copied = self
            .fs
            .read(rxfs_handle, offset, &mut buf)
            .map_err(map_fs_error)?;
        buf.truncate(copied);
        Ok(buf)
    }

    pub fn write(
        &mut self,
        owner: u32,
        handle: u32,
        offset: u64,
        payload: &[u8],
    ) -> Result<usize, VfsError> {
        let entry = self
            .opens
            .get(&handle)
            .filter(|entry| entry.owner == owner)
            .ok_or(VfsError::BadHandle)?;
        if self.rom_names.contains(entry.name.as_str()) {
            return Err(VfsError::NotPermitted);
        }
        if payload.is_empty() {
            return Ok(0);
        }
        if entry.mode & wire::MODE_WRITE == 0 {
            return Err(VfsError::NotPermitted);
        }
        let rxfs_handle = entry.rxfs_handle;
        let written = self
            .fs
            .write(rxfs_handle, offset, payload)
            .map_err(map_fs_error)?;
        self.fs.sync().map_err(map_fs_error)?;
        Ok(written)
    }

    pub fn close(&mut self, owner: u32, handle: u32) -> Result<(), VfsError> {
        let rxfs_handle = self.live_handle(owner, handle)?;
        self.fs.close(rxfs_handle).map_err(map_fs_error)?;
        self.opens.remove(&handle);
        Ok(())
    }

    pub fn delete(&mut self, name: &str) -> Result<(), VfsError> {
        if !wire::valid_name(name) {
            return Err(VfsError::InvalidPath);
        }
        if self.rom_names.contains(name) {
            return Err(VfsError::NotPermitted);
        }
        self.fs.remove(&volume_path(name)).map_err(map_fs_error)?;
        self.fs.sync().map_err(map_fs_error)
    }

    pub fn list(&mut self) -> Result<Vec<u8>, VfsError> {
        let items = self.fs.readdir(b"/").map_err(map_fs_error)?;
        let mut names: Vec<&[u8]> = items
            .iter()
            .map(|item| item.name.as_slice())
            .filter(|name| !dot_entry(name))
            .collect();
        names.sort_unstable();
        let mut blob = Vec::new();
        for name in names {
            if !blob.is_empty() {
                blob.push(b'\n');
            }
            blob.extend_from_slice(name);
        }
        if blob.len() > wire::RPC_MAX_DATA_BYTES {
            return Err(VfsError::ListTooLarge);
        }
        Ok(blob)
    }

    fn live_handle(&self, owner: u32, handle: u32) -> Result<Handle, VfsError> {
        match self.opens.get(&handle) {
            Some(entry) if entry.owner == owner => Ok(entry.rxfs_handle),
            _ => Err(VfsError::BadHandle),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::format;

    const OWNER: u32 = 0x0010_0001;
    const INTRUDER: u32 = 0x0020_0002;

    fn create(fs: &mut VfsStore, owner: u32, name: &str) -> u32 {
        fs.open(owner, wire::MODE_CREATE | wire::MODE_WRITE, name)
            .expect("create must succeed")
    }

    fn contents(fs: &mut VfsStore, owner: u32, handle: u32) -> Vec<u8> {
        fs.read(owner, handle, 0, wire::RPC_MAX_DATA_BYTES)
            .expect("read must succeed")
    }

    fn name_of_len(length: usize) -> String {
        (0..length).map(|_| 'k').collect()
    }

    #[test]
    fn volume_geometry_is_pinned() {
        assert_eq!(VOLUME_BLOCKS, 2048);
        let geometry = rxfs::spec::Geometry::plan(VOLUME_BLOCKS, DEFAULT_BYTES_PER_INODE).unwrap();
        assert_eq!(geometry.inode_count, 512);
        assert_eq!(geometry.data_blocks, 2005);
    }

    #[test]
    fn open_with_create_creates_empty_file_and_first_handle_is_one() {
        let mut fs = VfsStore::new();
        let handle = create(&mut fs, OWNER, "a.txt");
        assert_eq!(handle, 1);
        assert_eq!(contents(&mut fs, OWNER, handle), Vec::new());
    }

    #[test]
    fn open_existing_without_create_reuses_existing_content() {
        let mut fs = VfsStore::new();
        let first = create(&mut fs, OWNER, "a.txt");
        fs.write(OWNER, first, 0, b"abc").unwrap();
        fs.close(OWNER, first).unwrap();
        let second = fs.open(OWNER, 0, "a.txt").expect("plain open must succeed");
        assert_eq!(second, 2);
        assert_eq!(contents(&mut fs, OWNER, second), b"abc".to_vec());
    }

    #[test]
    fn open_existing_with_create_is_already_exists() {
        let mut fs = VfsStore::new();
        create(&mut fs, OWNER, "a.txt");
        assert_eq!(
            fs.open(OWNER, wire::MODE_CREATE, "a.txt"),
            Err(VfsError::AlreadyExists)
        );
    }

    #[test]
    fn open_missing_without_create_is_not_found() {
        let mut fs = VfsStore::new();
        assert_eq!(fs.open(OWNER, 0, "ghost.txt"), Err(VfsError::NotFound));
    }

    #[test]
    fn handles_increase_monotonically() {
        let mut fs = VfsStore::new();
        assert_eq!(create(&mut fs, OWNER, "a"), 1);
        assert_eq!(create(&mut fs, OWNER, "b"), 2);
        assert_eq!(create(&mut fs, OWNER, "c"), 3);
    }

    #[test]
    fn open_invalid_name_is_invalid_path() {
        let mut fs = VfsStore::new();
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
        let mut fs = VfsStore::new();
        assert_eq!(fs.delete("bad\nname"), Err(VfsError::InvalidPath));
        assert_eq!(fs.delete(""), Err(VfsError::InvalidPath));
    }

    #[test]
    fn read_at_end_is_empty() {
        let mut fs = VfsStore::new();
        let handle = create(&mut fs, OWNER, "a.txt");
        fs.write(OWNER, handle, 0, b"abc").unwrap();
        assert_eq!(fs.read(OWNER, handle, 3, 10).unwrap(), Vec::new());
    }

    #[test]
    fn read_past_end_is_empty() {
        let mut fs = VfsStore::new();
        let handle = create(&mut fs, OWNER, "a.txt");
        fs.write(OWNER, handle, 0, b"abc").unwrap();
        assert_eq!(fs.read(OWNER, handle, 999, 10).unwrap(), Vec::new());
    }

    #[test]
    fn read_clamps_across_end() {
        let mut fs = VfsStore::new();
        let handle = create(&mut fs, OWNER, "a.txt");
        fs.write(OWNER, handle, 0, b"abcde").unwrap();
        assert_eq!(fs.read(OWNER, handle, 3, 100).unwrap(), b"de".to_vec());
    }

    #[test]
    fn unknown_handle_is_bad_handle_for_every_operation() {
        let mut fs = VfsStore::new();
        assert_eq!(fs.read(OWNER, 42, 0, 10), Err(VfsError::BadHandle));
        assert_eq!(fs.write(OWNER, 42, 0, b"x"), Err(VfsError::BadHandle));
        assert_eq!(fs.close(OWNER, 42), Err(VfsError::BadHandle));
    }

    #[test]
    fn cross_owner_handle_use_is_bad_handle() {
        let mut fs = VfsStore::new();
        let handle = create(&mut fs, OWNER, "a.txt");
        fs.write(OWNER, handle, 0, b"secret").unwrap();
        assert_eq!(fs.read(INTRUDER, handle, 0, 10), Err(VfsError::BadHandle));
        assert_eq!(
            fs.write(INTRUDER, handle, 0, b"x"),
            Err(VfsError::BadHandle)
        );
        assert_eq!(fs.close(INTRUDER, handle), Err(VfsError::BadHandle));
        assert_eq!(contents(&mut fs, OWNER, handle), b"secret".to_vec());
        assert_eq!(fs.close(OWNER, handle), Ok(()));
    }

    #[test]
    fn write_without_write_mode_is_not_permitted() {
        let mut fs = VfsStore::new();
        let created = create(&mut fs, OWNER, "a.txt");
        fs.write(OWNER, created, 0, b"abc").unwrap();
        fs.close(OWNER, created).unwrap();
        let readonly = fs.open(OWNER, 0, "a.txt").unwrap();
        assert_eq!(
            fs.write(OWNER, readonly, 0, b"x"),
            Err(VfsError::NotPermitted)
        );
        assert_eq!(contents(&mut fs, OWNER, readonly), b"abc".to_vec());
    }

    #[test]
    fn read_is_permitted_on_mode_zero_handle() {
        let mut fs = VfsStore::new();
        let created = create(&mut fs, OWNER, "a.txt");
        fs.write(OWNER, created, 0, b"hello").unwrap();
        fs.close(OWNER, created).unwrap();
        let readonly = fs.open(OWNER, 0, "a.txt").unwrap();
        assert_eq!(contents(&mut fs, OWNER, readonly), b"hello".to_vec());
    }

    #[test]
    fn write_at_zero_into_empty_file() {
        let mut fs = VfsStore::new();
        let handle = create(&mut fs, OWNER, "a.txt");
        assert_eq!(fs.write(OWNER, handle, 0, b"hi"), Ok(2));
        assert_eq!(contents(&mut fs, OWNER, handle), b"hi".to_vec());
    }

    #[test]
    fn write_appending_at_len() {
        let mut fs = VfsStore::new();
        let handle = create(&mut fs, OWNER, "a.txt");
        fs.write(OWNER, handle, 0, b"ab").unwrap();
        assert_eq!(fs.write(OWNER, handle, 2, b"cd"), Ok(2));
        assert_eq!(contents(&mut fs, OWNER, handle), b"abcd".to_vec());
    }

    #[test]
    fn write_beyond_len_zero_fills_hole_byte_exactly() {
        let mut fs = VfsStore::new();
        let handle = create(&mut fs, OWNER, "a.txt");
        fs.write(OWNER, handle, 0, b"ab").unwrap();
        fs.write(OWNER, handle, 4, b"cd").unwrap();
        assert_eq!(
            contents(&mut fs, OWNER, handle),
            vec![b'a', b'b', 0, 0, b'c', b'd']
        );
    }

    #[test]
    fn empty_write_is_noop_and_never_resizes() {
        let mut fs = VfsStore::new();
        let handle = create(&mut fs, OWNER, "a.txt");
        fs.write(OWNER, handle, 0, b"abc").unwrap();
        assert_eq!(fs.write(OWNER, handle, 100, &[]), Ok(0));
        assert_eq!(contents(&mut fs, OWNER, handle), b"abc".to_vec());
    }

    #[test]
    fn close_frees_handle_and_second_close_is_bad_handle() {
        let mut fs = VfsStore::new();
        let handle = create(&mut fs, OWNER, "a.txt");
        assert_eq!(fs.close(OWNER, handle), Ok(()));
        assert_eq!(fs.close(OWNER, handle), Err(VfsError::BadHandle));
    }

    #[test]
    fn delete_then_read_via_still_open_handle_is_bad_handle() {
        let mut fs = VfsStore::new();
        let handle = create(&mut fs, OWNER, "a.txt");
        fs.write(OWNER, handle, 0, b"abc").unwrap();
        assert_eq!(fs.delete("a.txt"), Ok(()));
        assert_eq!(fs.read(OWNER, handle, 0, 10), Err(VfsError::BadHandle));
        assert_eq!(
            fs.write(OWNER, handle, 0, b"x"),
            Err(VfsError::BadHandle)
        );
    }

    #[test]
    fn handle_to_deleted_file_still_closes_cleanly() {
        let mut fs = VfsStore::new();
        let handle = create(&mut fs, OWNER, "a.txt");
        fs.delete("a.txt").unwrap();
        assert_eq!(fs.close(OWNER, handle), Ok(()));
    }

    #[test]
    fn delete_missing_is_not_found() {
        let mut fs = VfsStore::new();
        assert_eq!(fs.delete("ghost.txt"), Err(VfsError::NotFound));
    }

    #[test]
    fn list_returns_sorted_newline_joined_names() {
        let mut fs = VfsStore::new();
        create(&mut fs, OWNER, "notes.txt");
        create(&mut fs, OWNER, "blob.bin");
        assert_eq!(fs.list().unwrap(), b"blob.bin\nnotes.txt".to_vec());
    }

    #[test]
    fn list_empty_store_is_empty() {
        let mut fs = VfsStore::new();
        assert_eq!(fs.list().unwrap(), Vec::new());
    }

    #[test]
    fn two_handles_on_one_file_see_shared_content() {
        let mut fs = VfsStore::new();
        let writer = create(&mut fs, OWNER, "a.txt");
        let reader = fs.open(OWNER, 0, "a.txt").unwrap();
        fs.write(OWNER, writer, 0, b"shared").unwrap();
        assert_eq!(contents(&mut fs, OWNER, reader), b"shared".to_vec());
        assert_eq!(contents(&mut fs, OWNER, writer), b"shared".to_vec());
        fs.write(OWNER, reader, 0, b"x")
            .expect_err("mode zero must reject write");
    }

    #[test]
    fn open_beyond_engine_slot_cap_is_server_resource() {
        let mut fs = VfsStore::new();
        let mut held = Vec::new();
        for index in 0..64u32 {
            held.push(create(&mut fs, OWNER, &format!("s{index}")));
        }
        assert_eq!(
            fs.open(OWNER, wire::MODE_CREATE | wire::MODE_WRITE, "overflow"),
            Err(VfsError::ServerResource)
        );
        assert_eq!(fs.close(OWNER, held[0]), Ok(()));
        let revived = fs
            .open(OWNER, wire::MODE_CREATE | wire::MODE_WRITE, "spare")
            .expect("closing an open must free an engine slot");
        assert!(revived > 64);
    }

    #[test]
    fn create_beyond_inode_capacity_is_server_resource() {
        let mut fs = VfsStore::new();
        let mut created = 0u32;
        loop {
            let name = format!("exhaust{created}");
            let handle = match fs.open(OWNER, wire::MODE_CREATE, &name) {
                Ok(handle) => handle,
                Err(error) => {
                    assert_eq!(error, VfsError::ServerResource);
                    break;
                }
            };
            fs.close(OWNER, handle).unwrap();
            created += 1;
            if created > 600 {
                panic!("expected the 512-inode volume to run out of inodes");
            }
        }
        assert_eq!(created, 510);
        let blob = fs.list().unwrap();
        assert!(blob.len() > 510 * 8);
    }

    fn fabricated_rom() -> Vec<u8> {
        system::rom::encode(&[
            ("beta.bin", b"beta-payload".as_slice()),
            ("alpha.bin", b"alpha".as_slice()),
        ])
    }

    #[test]
    fn attach_rom_lists_rom_names_sorted_with_ram_names() {
        let mut fs = VfsStore::new();
        fs.attach_rom(&fabricated_rom()).unwrap();
        create(&mut fs, OWNER, "mid.txt");
        assert_eq!(fs.list().unwrap(), b"alpha.bin\nbeta.bin\nmid.txt".to_vec());
    }

    #[test]
    fn rom_file_reads_exact_bytes() {
        let mut fs = VfsStore::new();
        fs.attach_rom(&fabricated_rom()).unwrap();
        let handle = fs.open(OWNER, 0, "alpha.bin").unwrap();
        assert_eq!(contents(&mut fs, OWNER, handle), b"alpha".to_vec());
        assert_eq!(fs.read(OWNER, handle, 2, 2).unwrap(), b"ph".to_vec());
    }

    #[test]
    fn rom_read_clamps_across_window_end_and_eof() {
        let mut fs = VfsStore::new();
        fs.attach_rom(&fabricated_rom()).unwrap();
        let handle = fs.open(OWNER, 0, "beta.bin").unwrap();
        assert_eq!(fs.read(OWNER, handle, 5, 100).unwrap(), b"payload".to_vec());
        assert_eq!(fs.read(OWNER, handle, 12, 10).unwrap(), Vec::new());
        assert_eq!(fs.read(OWNER, handle, 999, 10).unwrap(), Vec::new());
    }

    #[test]
    fn open_rom_with_create_is_already_exists() {
        let mut fs = VfsStore::new();
        fs.attach_rom(&fabricated_rom()).unwrap();
        assert_eq!(
            fs.open(OWNER, wire::MODE_CREATE, "alpha.bin"),
            Err(VfsError::AlreadyExists)
        );
    }

    #[test]
    fn write_to_rom_file_is_not_permitted_even_when_empty() {
        let mut fs = VfsStore::new();
        fs.attach_rom(&fabricated_rom()).unwrap();
        let handle = fs.open(OWNER, wire::MODE_WRITE, "alpha.bin").unwrap();
        assert_eq!(
            fs.write(OWNER, handle, 0, b"x"),
            Err(VfsError::NotPermitted)
        );
        assert_eq!(
            fs.write(OWNER, handle, 0, &[]),
            Err(VfsError::NotPermitted)
        );
        assert_eq!(
            fs.write(OWNER, handle, 99, &[]),
            Err(VfsError::NotPermitted)
        );
        assert_eq!(contents(&mut fs, OWNER, handle), b"alpha".to_vec());
    }

    #[test]
    fn delete_rom_file_is_not_permitted() {
        let mut fs = VfsStore::new();
        fs.attach_rom(&fabricated_rom()).unwrap();
        assert_eq!(fs.delete("alpha.bin"), Err(VfsError::NotPermitted));
        assert_eq!(fs.list().unwrap(), b"alpha.bin\nbeta.bin".to_vec());
    }

    #[test]
    fn second_attach_rom_is_already_attached() {
        let mut fs = VfsStore::new();
        fs.attach_rom(&fabricated_rom()).unwrap();
        assert!(matches!(
            fs.attach_rom(&fabricated_rom()),
            Err(RomMountError::AlreadyAttached)
        ));
    }

    #[test]
    fn attach_rom_rejects_format_invalid_image() {
        let mut fs = VfsStore::new();
        assert!(matches!(
            fs.attach_rom(b"not a rom image at all"),
            Err(RomMountError::Format(RomError::BadMagic))
        ));
        assert_eq!(fs.list().unwrap(), Vec::new());
    }

    #[test]
    fn attach_rom_rejects_name_invalid_for_vfs() {
        let mut fs = VfsStore::new();
        let image = system::rom::encode(&[("bad\nname", b"x".as_slice())]);
        assert!(matches!(
            fs.attach_rom(&image),
            Err(RomMountError::InvalidName(ref name)) if name == "bad\nname"
        ));
        assert_eq!(fs.list().unwrap(), Vec::new());
    }

    #[test]
    fn attach_rom_rejects_collision_and_stores_nothing() {
        let mut fs = VfsStore::new();
        let handle = create(&mut fs, OWNER, "alpha.bin");
        assert!(matches!(
            fs.attach_rom(&fabricated_rom()),
            Err(RomMountError::Duplicate(ref name)) if name == "alpha.bin"
        ));
        assert_eq!(fs.list().unwrap(), b"alpha.bin".to_vec());
        fs.write(OWNER, handle, 0, b"still writable")
            .expect("collision rejection must not convert RAM to ROM");
        assert_eq!(contents(&mut fs, OWNER, handle), b"still writable".to_vec());
    }

    fn fat_fabricated_rom() -> Vec<u8> {
        let first: Vec<u8> = (0..5000u32).map(|i| (i % 253) as u8).collect();
        let second: Vec<u8> = (0..4097u32).map(|i| (i % 197) as u8).collect();
        system::rom::encode(&[("alpha.bin", first.as_slice()), ("beta.bin", second.as_slice())])
    }

    #[test]
    fn rom_ingest_is_byte_exact_across_blocks() {
        let mut fs = VfsStore::new();
        fs.attach_rom(&fat_fabricated_rom()).unwrap();
        let alpha = fs.open(OWNER, 0, "alpha.bin").unwrap();
        let expected_alpha: Vec<u8> = (0..5000u32).map(|i| (i % 253) as u8).collect();
        assert_eq!(contents(&mut fs, OWNER, alpha), expected_alpha);
        let beta = fs.open(OWNER, 0, "beta.bin").unwrap();
        let expected_beta: Vec<u8> = (0..4097u32).map(|i| (i % 197) as u8).collect();
        assert_eq!(contents(&mut fs, OWNER, beta), expected_beta);
        assert_eq!(fs.list().unwrap(), b"alpha.bin\nbeta.bin".to_vec());
    }
}
