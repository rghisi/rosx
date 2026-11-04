use crate::hdd::AtaDisk;
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use ata_protocol::client::{Ata, ClientError};
use ata_protocol::wire::AtaError;
use fs_core::engine::{Engine, Kind, MetadataView};
use fs_core::error::FsError;
use rxfs::device::BlockDevice;
use rxfs::device::MemoryDevice;
use rxfs::spec::DEFAULT_BYTES_PER_INODE;
use rxfs::superblock::UUID_BYTES;
use rxfs::volume::{FileSystem, FormatParams, Handle as RxfsHandle};
use system::rom::{RomError, RomImage};
use vfs_protocol::wire::{self, EntityKind, VfsError};

const VOLUME_BLOCKS: u64 = 2048;
const VOLUME_UUID: [u8; UUID_BYTES] = *b"rosx-ram-vol-001";
const VOLUME_NAME: &[u8] = b"rosx-ram";
const HDD_VOLUME_UUID: [u8; UUID_BYTES] = *b"rosx-ata-vol-001";
const HDD_VOLUME_NAME: &[u8] = b"rosx-hdd";
const SEEDED_DIRECTORIES: [&str; 4] = ["tests", "services", "games", "bench"];

pub type ActiveVolume = FileSystem<MemoryDevice>;
pub type ActiveStore = VfsStore<ActiveVolume, AtaDisk>;

struct MountedVolume<D: BlockDevice> {
    name: String,
    drive: u8,
    fs: FileSystem<D>,
}

enum OpenLocation<E: Engine> {
    Root(E::Handle),
    Mount { slot: usize, handle: RxfsHandle },
}

impl<E: Engine> Copy for OpenLocation<E> {}
impl<E: Engine> Clone for OpenLocation<E> {
    fn clone(&self) -> Self {
        *self
    }
}

struct OpenEntry<E: Engine> {
    owner: u32,
    mode: u8,
    location: OpenLocation<E>,
}

#[derive(Debug)]
pub enum RomMountError {
    Format(RomError),
    InvalidPath(String),
    Duplicate(String),
    AlreadyAttached,
    Engine(FsError),
}

pub struct VfsStore<E: Engine, D: BlockDevice> {
    engine: E,
    mounts: Vec<MountedVolume<D>>,
    opens: BTreeMap<u32, OpenEntry<E>>,
    next_handle: u32,
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

fn is_strict_ancestor(ancestor: &str, path: &str) -> bool {
    path.len() > ancestor.len()
        && path.starts_with(ancestor)
        && path.as_bytes()[ancestor.len()] == b'/'
}

fn ancestor_paths(path: &str) -> Vec<&str> {
    let mut ancestors = Vec::new();
    let mut head = path;
    while let Some((parent, _)) = head.rsplit_once('/') {
        ancestors.push(parent);
        head = parent;
    }
    ancestors.reverse();
    ancestors
}

fn map_fs_error(error: FsError) -> VfsError {
    match error {
        FsError::NotFound => VfsError::NotFound,
        FsError::AlreadyExists => VfsError::AlreadyExists,
        FsError::InvalidPath | FsError::PathTooLong | FsError::NameTooLong | FsError::InvalidName => {
            VfsError::InvalidPath
        }
        FsError::BadHandle => VfsError::BadHandle,
        FsError::NotADirectory => VfsError::NotDirectory,
        FsError::IsADirectory => VfsError::IsADirectory,
        FsError::DirectoryNotEmpty => VfsError::DirectoryNotEmpty,
        _ => VfsError::ServerResource,
    }
}

fn map_ata_error(error: ClientError) -> VfsError {
    match error {
        ClientError::Ata(AtaError::NotFound) => VfsError::NotFound,
        _ => VfsError::ServerResource,
    }
}

fn map_mount_error(error: FsError) -> VfsError {
    match error {
        FsError::Corrupt | FsError::DeviceTooSmall => VfsError::NotFormatted,
        other => map_fs_error(other),
    }
}

fn names_blob(names: &[&[u8]]) -> Result<Vec<u8>, VfsError> {
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

enum Target {
    Root,
    Volume(usize, String),
}

fn open_engine<E2: Engine>(
    engine: &mut E2,
    mode: u8,
    path: &[u8],
) -> Result<E2::Handle, VfsError> {
    match engine.lookup(path) {
        Ok(_) if mode & wire::MODE_CREATE != 0 => Err(VfsError::AlreadyExists),
        Ok(_) => engine.open(path).map_err(map_fs_error),
        Err(FsError::NotFound) if mode & wire::MODE_CREATE != 0 => {
            let handle = engine.create(path).map_err(map_fs_error)?;
            engine.sync().map_err(map_fs_error)?;
            Ok(handle)
        }
        Err(FsError::NotFound) => Err(VfsError::NotFound),
        Err(error) => Err(map_fs_error(error)),
    }
}

impl VfsStore<ActiveVolume, AtaDisk> {
    pub fn new() -> VfsStore<ActiveVolume, AtaDisk> {
        let params = FormatParams {
            bytes_per_inode: DEFAULT_BYTES_PER_INODE,
            uuid: VOLUME_UUID,
            volume_name: VOLUME_NAME,
            created_at: 0,
        };
        let mut engine = FileSystem::format(MemoryDevice::new(VOLUME_BLOCKS), &params)
            .expect("fresh memory volume must format");
        for seed in SEEDED_DIRECTORIES {
            engine
                .mkdir(&volume_path(seed))
                .expect("seed directory on a fresh volume must be creatable");
        }
        VfsStore::from_engine(engine)
    }

    pub fn format_drive(&mut self, drive: u8) -> Result<(), VfsError> {
        if self.mounts.iter().any(|mount| mount.drive == drive) {
            return Err(VfsError::Busy);
        }
        let ata = Ata::connect().map_err(map_ata_error)?;
        let identity = ata.identify(drive).map_err(map_ata_error)?;
        let params = FormatParams {
            bytes_per_inode: DEFAULT_BYTES_PER_INODE,
            uuid: HDD_VOLUME_UUID,
            volume_name: HDD_VOLUME_NAME,
            created_at: 0,
        };
        let fs = FileSystem::format(AtaDisk::new(ata, drive, identity.sectors), &params)
            .map_err(map_fs_error)?;
        fs.unmount()
            .map(|disk| disk.into_ata().disconnect())
            .map_err(map_fs_error)
    }

    pub fn mount_drive(&mut self, drive: u8, dir: &str) -> Result<(), VfsError> {
        if !wire::valid_name(dir) {
            return Err(VfsError::InvalidPath);
        }
        if self.mounts.iter().any(|mount| mount.drive == drive)
            || self.mounts.iter().any(|mount| mount.name == dir)
        {
            return Err(VfsError::Busy);
        }
        let mount_point = self
            .engine
            .lookup(&volume_path(dir))
            .map_err(map_fs_error)?;
        if mount_point.kind() != Kind::Directory {
            return Err(VfsError::NotDirectory);
        }
        let ata = Ata::connect().map_err(map_ata_error)?;
        let identity = ata.identify(drive).map_err(map_ata_error)?;
        let fs =
            FileSystem::mount(AtaDisk::new(ata, drive, identity.sectors)).map_err(map_mount_error)?;
        self.mounts.push(MountedVolume {
            name: String::from(dir),
            drive,
            fs,
        });
        Ok(())
    }
}

impl<E: Engine, D: BlockDevice> VfsStore<E, D> {
    fn from_engine(engine: E) -> VfsStore<E, D> {
        VfsStore {
            engine,
            mounts: Vec::new(),
            opens: BTreeMap::new(),
            next_handle: 1,
            rom_attached: false,
        }
    }

    fn route(&self, path: &str) -> Result<Target, VfsError> {
        let (head, rest) = match path.split_once('/') {
            Some(split) => split,
            None => (path, ""),
        };
        match self.mounts.iter().position(|mount| mount.name == head) {
            Some(slot) => Ok(Target::Volume(slot, format!("/{rest}"))),
            None => Ok(Target::Root),
        }
    }

    fn volume_mut(&mut self, slot: usize) -> Result<&mut FileSystem<D>, VfsError> {
        self.mounts
            .get_mut(slot)
            .map(|mounted| &mut mounted.fs)
            .ok_or(VfsError::BadHandle)
    }

    fn live_location(&self, owner: u32, handle: u32) -> Result<OpenLocation<E>, VfsError> {
        match self.opens.get(&handle) {
            Some(entry) if entry.owner == owner => Ok(entry.location),
            _ => Err(VfsError::BadHandle),
        }
    }

    fn is_mount_point(&self, name: &str) -> bool {
        self.mounts.iter().any(|mount| mount.name == name)
    }

    pub fn attach_rom(&mut self, image: &[u8]) -> Result<(), RomMountError> {
        if self.rom_attached {
            return Err(RomMountError::AlreadyAttached);
        }
        let parsed = RomImage::parse(image).map_err(RomMountError::Format)?;
        let entries: Vec<_> = parsed.files().collect();
        let mut staged: Vec<(&str, usize, usize)> = Vec::new();
        for entry in &entries {
            if !wire::valid_path(entry.name) {
                return Err(RomMountError::InvalidPath(String::from(entry.name)));
            }
            if self.engine.lookup(&volume_path(entry.name)).is_ok() {
                return Err(RomMountError::Duplicate(String::from(entry.name)));
            }
            if let Some(other) = entries.iter().find(|other| {
                other.name != entry.name
                    && (is_strict_ancestor(entry.name, other.name)
                        || is_strict_ancestor(other.name, entry.name))
            }) {
                let shorter = if entry.name.len() <= other.name.len() {
                    entry.name
                } else {
                    other.name
                };
                return Err(RomMountError::Duplicate(String::from(shorter)));
            }
            for ancestor in ancestor_paths(entry.name) {
                match self.engine.lookup(&volume_path(ancestor)) {
                    Ok(stat) if stat.kind() == Kind::Directory => {}
                    Ok(_) => return Err(RomMountError::Duplicate(String::from(ancestor))),
                    Err(FsError::NotFound) => {}
                    Err(error) => return Err(RomMountError::Engine(error)),
                }
            }
            staged.push((entry.name, entry.offset, entry.len));
        }
        for (name, offset, len) in staged {
            for ancestor in ancestor_paths(name) {
                match self.engine.mkdir(&volume_path(ancestor)) {
                    Ok(()) | Err(FsError::AlreadyExists) => {}
                    Err(error) => return Err(RomMountError::Engine(error)),
                }
            }
            let handle = self
                .engine
                .create(&volume_path(name))
                .map_err(RomMountError::Engine)?;
            if len > 0 {
                self.engine
                    .write(handle, 0, &image[offset..offset + len])
                    .map_err(RomMountError::Engine)?;
            }
            self.engine.close(handle).map_err(RomMountError::Engine)?;
        }
        self.engine.sync().map_err(RomMountError::Engine)?;
        self.rom_attached = true;
        Ok(())
    }

    pub fn open(&mut self, owner: u32, mode: u8, name: &str) -> Result<u32, VfsError> {
        if !wire::valid_path(name) {
            return Err(VfsError::InvalidPath);
        }
        let location = match self.route(name)? {
            Target::Root => {
                let handle = open_engine(&mut self.engine, mode, &volume_path(name))?;
                OpenLocation::Root(handle)
            }
            Target::Volume(slot, path) => {
                let handle = open_engine(self.volume_mut(slot)?, mode, path.as_bytes())?;
                OpenLocation::Mount { slot, handle }
            }
        };
        let handle = self.next_handle;
        self.next_handle += 1;
        self.opens.insert(
            handle,
            OpenEntry {
                owner,
                mode,
                location,
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
        let location = self.live_location(owner, handle)?;
        let mut buf = vec![0u8; count];
        let copied = match location {
            OpenLocation::Root(engine_handle) => self.engine.read(engine_handle, offset, &mut buf),
            OpenLocation::Mount { slot, handle } => {
                self.volume_mut(slot)?.read(handle, offset, &mut buf)
            }
        }
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
        if payload.is_empty() {
            return Ok(0);
        }
        if entry.mode & wire::MODE_WRITE == 0 {
            return Err(VfsError::NotPermitted);
        }
        let location = entry.location;
        let written = match location {
            OpenLocation::Root(engine_handle) => {
                let written = self
                    .engine
                    .write(engine_handle, offset, payload)
                    .map_err(map_fs_error)?;
                self.engine.sync().map_err(map_fs_error)?;
                written
            }
            OpenLocation::Mount { slot, handle } => {
                let volume = self.volume_mut(slot)?;
                let written = volume.write(handle, offset, payload).map_err(map_fs_error)?;
                volume.sync().map_err(map_fs_error)?;
                written
            }
        };
        Ok(written)
    }

    pub fn close(&mut self, owner: u32, handle: u32) -> Result<(), VfsError> {
        let location = self.live_location(owner, handle)?;
        match location {
            OpenLocation::Root(engine_handle) => {
                self.engine.close(engine_handle).map_err(map_fs_error)?
            }
            OpenLocation::Mount { slot, handle } => {
                self.volume_mut(slot)?.close(handle).map_err(map_fs_error)?
            }
        }
        self.opens.remove(&handle);
        Ok(())
    }

    pub fn delete(&mut self, name: &str) -> Result<(), VfsError> {
        if !wire::valid_path(name) {
            return Err(VfsError::InvalidPath);
        }
        if self.is_mount_point(name) {
            return Err(VfsError::NotPermitted);
        }
        match self.route(name)? {
            Target::Root => {
                self.engine.remove(&volume_path(name)).map_err(map_fs_error)?;
                self.engine.sync().map_err(map_fs_error)
            }
            Target::Volume(slot, path) => {
                let volume = self.volume_mut(slot)?;
                volume.remove(path.as_bytes()).map_err(map_fs_error)?;
                volume.sync().map_err(map_fs_error)
            }
        }
    }

    pub fn mkdir(&mut self, path: &str) -> Result<(), VfsError> {
        if !wire::valid_path(path) {
            return Err(VfsError::InvalidPath);
        }
        match self.route(path)? {
            Target::Root => {
                self.engine.mkdir(&volume_path(path)).map_err(map_fs_error)?;
                self.engine.sync().map_err(map_fs_error)
            }
            Target::Volume(slot, path) => {
                let volume = self.volume_mut(slot)?;
                volume.mkdir(path.as_bytes()).map_err(map_fs_error)?;
                volume.sync().map_err(map_fs_error)
            }
        }
    }

    pub fn rmdir(&mut self, path: &str) -> Result<(), VfsError> {
        if !wire::valid_path(path) {
            return Err(VfsError::InvalidPath);
        }
        if self.is_mount_point(path) {
            return Err(VfsError::NotPermitted);
        }
        match self.route(path)? {
            Target::Root => {
                self.engine.rmdir(&volume_path(path)).map_err(map_fs_error)?;
                self.engine.sync().map_err(map_fs_error)
            }
            Target::Volume(slot, path) => {
                let volume = self.volume_mut(slot)?;
                volume.rmdir(path.as_bytes()).map_err(map_fs_error)?;
                volume.sync().map_err(map_fs_error)
            }
        }
    }

    pub fn stat(&mut self, name: &str) -> Result<(EntityKind, u64), VfsError> {
        if !wire::valid_path(name) {
            return Err(VfsError::InvalidPath);
        }
        let (kind, size) = match self.route(name)? {
            Target::Root => {
                let stat = self.engine.lookup(&volume_path(name)).map_err(map_fs_error)?;
                (stat.kind(), stat.size())
            }
            Target::Volume(slot, path) => {
                let stat = self
                    .volume_mut(slot)?
                    .lookup(path.as_bytes())
                    .map_err(map_fs_error)?;
                (stat.kind(), stat.size())
            }
        };
        let kind = match kind {
            Kind::Regular => EntityKind::File,
            Kind::Directory => EntityKind::Directory,
        };
        Ok((kind, size))
    }

    pub fn list(&mut self) -> Result<Vec<u8>, VfsError> {
        let items = self.engine.readdir(b"/").map_err(map_fs_error)?;
        let mut names: Vec<&[u8]> = items
            .iter()
            .map(|item| item.name.as_slice())
            .filter(|name| !dot_entry(name))
            .collect();
        names.sort_unstable();
        names_blob(&names)
    }

    pub fn list_dir(&mut self, dir: &str) -> Result<Vec<u8>, VfsError> {
        if !wire::valid_path(dir) {
            return Err(VfsError::InvalidPath);
        }
        let items = match self.route(dir)? {
            Target::Root => self.engine.readdir(&volume_path(dir)).map_err(map_fs_error)?,
            Target::Volume(slot, path) => self
                .volume_mut(slot)?
                .readdir(path.as_bytes())
                .map_err(map_fs_error)?,
        };
        let mut names: Vec<&[u8]> = items
            .iter()
            .map(|item| item.name.as_slice())
            .filter(|name| !dot_entry(name))
            .collect();
        names.sort_unstable();
        names_blob(&names)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OWNER: u32 = 0x0010_0001;

    fn fresh_store() -> VfsStore<ActiveVolume, MemoryDevice> {
        let params = FormatParams {
            bytes_per_inode: DEFAULT_BYTES_PER_INODE,
            uuid: VOLUME_UUID,
            volume_name: VOLUME_NAME,
            created_at: 0,
        };
        let engine = FileSystem::format(MemoryDevice::new(VOLUME_BLOCKS), &params)
            .expect("fresh memory volume must format");
        VfsStore::from_engine(engine)
    }

    fn mount_fake(fs: &mut VfsStore<ActiveVolume, MemoryDevice>, name: &str) {
        fs.mkdir(name).unwrap();
        let params = FormatParams {
            bytes_per_inode: DEFAULT_BYTES_PER_INODE,
            uuid: HDD_VOLUME_UUID,
            volume_name: HDD_VOLUME_NAME,
            created_at: 0,
        };
        let volume = FileSystem::format(MemoryDevice::new(64), &params).unwrap();
        fs.mounts.push(MountedVolume {
            name: String::from(name),
            drive: 9,
            fs: volume,
        });
    }

    const INTRUDER: u32 = 0x0020_0002;

    fn create<E: Engine, D: BlockDevice>(fs: &mut VfsStore<E, D>, owner: u32, name: &str) -> u32 {
        fs.open(owner, wire::MODE_CREATE | wire::MODE_WRITE, name)
            .expect("create must succeed")
    }

    fn contents<E: Engine, D: BlockDevice>(fs: &mut VfsStore<E, D>, owner: u32, handle: u32) -> Vec<u8> {
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
        let mut fs = fresh_store();
        let handle = create(&mut fs, OWNER, "a.txt");
        assert_eq!(handle, 1);
        assert_eq!(contents(&mut fs, OWNER, handle), Vec::new());
    }

    #[test]
    fn open_existing_without_create_reuses_existing_content() {
        let mut fs = fresh_store();
        let first = create(&mut fs, OWNER, "a.txt");
        fs.write(OWNER, first, 0, b"abc").unwrap();
        fs.close(OWNER, first).unwrap();
        let second = fs.open(OWNER, 0, "a.txt").expect("plain open must succeed");
        assert_eq!(second, 2);
        assert_eq!(contents(&mut fs, OWNER, second), b"abc".to_vec());
    }

    #[test]
    fn open_existing_with_create_is_already_exists() {
        let mut fs = fresh_store();
        create(&mut fs, OWNER, "a.txt");
        assert_eq!(
            fs.open(OWNER, wire::MODE_CREATE, "a.txt"),
            Err(VfsError::AlreadyExists)
        );
    }

    #[test]
    fn open_missing_without_create_is_not_found() {
        let mut fs = fresh_store();
        assert_eq!(fs.open(OWNER, 0, "ghost.txt"), Err(VfsError::NotFound));
    }

    #[test]
    fn handles_increase_monotonically() {
        let mut fs = fresh_store();
        assert_eq!(create(&mut fs, OWNER, "a"), 1);
        assert_eq!(create(&mut fs, OWNER, "b"), 2);
        assert_eq!(create(&mut fs, OWNER, "c"), 3);
    }

    #[test]
    fn open_invalid_name_is_invalid_path() {
        let mut fs = fresh_store();
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
        let mut fs = fresh_store();
        assert_eq!(fs.delete("bad\nname"), Err(VfsError::InvalidPath));
        assert_eq!(fs.delete(""), Err(VfsError::InvalidPath));
    }

    #[test]
    fn read_at_end_is_empty() {
        let mut fs = fresh_store();
        let handle = create(&mut fs, OWNER, "a.txt");
        fs.write(OWNER, handle, 0, b"abc").unwrap();
        assert_eq!(fs.read(OWNER, handle, 3, 10).unwrap(), Vec::new());
    }

    #[test]
    fn read_past_end_is_empty() {
        let mut fs = fresh_store();
        let handle = create(&mut fs, OWNER, "a.txt");
        fs.write(OWNER, handle, 0, b"abc").unwrap();
        assert_eq!(fs.read(OWNER, handle, 999, 10).unwrap(), Vec::new());
    }

    #[test]
    fn read_clamps_across_end() {
        let mut fs = fresh_store();
        let handle = create(&mut fs, OWNER, "a.txt");
        fs.write(OWNER, handle, 0, b"abcde").unwrap();
        assert_eq!(fs.read(OWNER, handle, 3, 100).unwrap(), b"de".to_vec());
    }

    #[test]
    fn unknown_handle_is_bad_handle_for_every_operation() {
        let mut fs = fresh_store();
        assert_eq!(fs.read(OWNER, 42, 0, 10), Err(VfsError::BadHandle));
        assert_eq!(fs.write(OWNER, 42, 0, b"x"), Err(VfsError::BadHandle));
        assert_eq!(fs.close(OWNER, 42), Err(VfsError::BadHandle));
    }

    #[test]
    fn cross_owner_handle_use_is_bad_handle() {
        let mut fs = fresh_store();
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
        let mut fs = fresh_store();
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
        let mut fs = fresh_store();
        let created = create(&mut fs, OWNER, "a.txt");
        fs.write(OWNER, created, 0, b"hello").unwrap();
        fs.close(OWNER, created).unwrap();
        let readonly = fs.open(OWNER, 0, "a.txt").unwrap();
        assert_eq!(contents(&mut fs, OWNER, readonly), b"hello".to_vec());
    }

    #[test]
    fn write_at_zero_into_empty_file() {
        let mut fs = fresh_store();
        let handle = create(&mut fs, OWNER, "a.txt");
        assert_eq!(fs.write(OWNER, handle, 0, b"hi"), Ok(2));
        assert_eq!(contents(&mut fs, OWNER, handle), b"hi".to_vec());
    }

    #[test]
    fn write_appending_at_len() {
        let mut fs = fresh_store();
        let handle = create(&mut fs, OWNER, "a.txt");
        fs.write(OWNER, handle, 0, b"ab").unwrap();
        assert_eq!(fs.write(OWNER, handle, 2, b"cd"), Ok(2));
        assert_eq!(contents(&mut fs, OWNER, handle), b"abcd".to_vec());
    }

    #[test]
    fn write_beyond_len_zero_fills_hole_byte_exactly() {
        let mut fs = fresh_store();
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
        let mut fs = fresh_store();
        let handle = create(&mut fs, OWNER, "a.txt");
        fs.write(OWNER, handle, 0, b"abc").unwrap();
        assert_eq!(fs.write(OWNER, handle, 100, &[]), Ok(0));
        assert_eq!(contents(&mut fs, OWNER, handle), b"abc".to_vec());
    }

    #[test]
    fn close_frees_handle_and_second_close_is_bad_handle() {
        let mut fs = fresh_store();
        let handle = create(&mut fs, OWNER, "a.txt");
        assert_eq!(fs.close(OWNER, handle), Ok(()));
        assert_eq!(fs.close(OWNER, handle), Err(VfsError::BadHandle));
    }

    #[test]
    fn delete_then_read_via_still_open_handle_is_bad_handle() {
        let mut fs = fresh_store();
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
        let mut fs = fresh_store();
        let handle = create(&mut fs, OWNER, "a.txt");
        fs.delete("a.txt").unwrap();
        assert_eq!(fs.close(OWNER, handle), Ok(()));
    }

    #[test]
    fn delete_missing_is_not_found() {
        let mut fs = fresh_store();
        assert_eq!(fs.delete("ghost.txt"), Err(VfsError::NotFound));
    }

    #[test]
    fn stat_missing_is_not_found() {
        let mut fs = fresh_store();
        assert_eq!(fs.stat("ghost.txt"), Err(VfsError::NotFound));
    }

    #[test]
    fn stat_created_empty_file_is_zero_size() {
        let mut fs = fresh_store();
        create(&mut fs, OWNER, "a.txt");
        assert_eq!(fs.stat("a.txt").unwrap(), (EntityKind::File, 0));
    }

    #[test]
    fn stat_tracks_writes_and_holes() {
        let mut fs = fresh_store();
        let handle = create(&mut fs, OWNER, "a.txt");
        fs.write(OWNER, handle, 0, b"ab").unwrap();
        fs.write(OWNER, handle, 4096, b"c").unwrap();
        assert_eq!(fs.stat("a.txt").unwrap(), (EntityKind::File, 4097));
    }

    #[test]
    fn stat_of_deleted_name_is_not_found() {
        let mut fs = fresh_store();
        create(&mut fs, OWNER, "a.txt");
        fs.delete("a.txt").unwrap();
        assert_eq!(fs.stat("a.txt"), Err(VfsError::NotFound));
    }

    #[test]
    fn stat_invalid_name_is_invalid_path() {
        let mut fs = fresh_store();
        assert_eq!(fs.stat(""), Err(VfsError::InvalidPath));
        assert_eq!(fs.stat("bad//name"), Err(VfsError::InvalidPath));
        assert_eq!(fs.stat("hdd/x"), Err(VfsError::NotFound));
        assert_eq!(fs.stat("."), Err(VfsError::InvalidPath));
        assert_eq!(fs.stat(".."), Err(VfsError::InvalidPath));
    }

    #[test]
    fn stat_succeeds_when_all_engine_slots_are_open() {
        let mut fs = fresh_store();
        create(&mut fs, OWNER, "probe.txt");
        for index in 0..63u32 {
            create(&mut fs, OWNER, &format!("s{index}"));
        }
        assert_eq!(
            fs.open(OWNER, wire::MODE_CREATE, "overflow"),
            Err(VfsError::ServerResource)
        );
        assert_eq!(fs.stat("probe.txt").unwrap(), (EntityKind::File, 0));
    }

    #[test]
    fn rom_file_stat_reports_exact_length() {
        let mut fs = fresh_store();
        fs.attach_rom(&fabricated_rom()).unwrap();
        assert_eq!(fs.stat("alpha.bin").unwrap(), (EntityKind::File, 5));
        assert_eq!(fs.stat("beta.bin").unwrap(), (EntityKind::File, 12));
    }

    #[test]
    fn list_returns_sorted_newline_joined_names() {
        let mut fs = fresh_store();
        create(&mut fs, OWNER, "notes.txt");
        create(&mut fs, OWNER, "blob.bin");
        assert_eq!(fs.list().unwrap(), b"blob.bin\nnotes.txt".to_vec());
    }

    #[test]
    fn list_of_empty_volume_is_empty() {
        let mut fs = fresh_store();
        assert_eq!(fs.list().unwrap(), Vec::new());
    }

    #[test]
    fn two_handles_on_one_file_see_shared_content() {
        let mut fs = fresh_store();
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
        let mut fs = fresh_store();
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
        let mut fs = fresh_store();
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
        let mut fs = fresh_store();
        fs.attach_rom(&fabricated_rom()).unwrap();
        create(&mut fs, OWNER, "mid.txt");
        assert_eq!(
            fs.list().unwrap(),
            b"alpha.bin\nbeta.bin\nmid.txt".to_vec()
        );
    }

    #[test]
    fn rom_file_reads_exact_bytes() {
        let mut fs = fresh_store();
        fs.attach_rom(&fabricated_rom()).unwrap();
        let handle = fs.open(OWNER, 0, "alpha.bin").unwrap();
        assert_eq!(contents(&mut fs, OWNER, handle), b"alpha".to_vec());
        assert_eq!(fs.read(OWNER, handle, 2, 2).unwrap(), b"ph".to_vec());
    }

    #[test]
    fn rom_read_clamps_across_window_end_and_eof() {
        let mut fs = fresh_store();
        fs.attach_rom(&fabricated_rom()).unwrap();
        let handle = fs.open(OWNER, 0, "beta.bin").unwrap();
        assert_eq!(fs.read(OWNER, handle, 5, 100).unwrap(), b"payload".to_vec());
        assert_eq!(fs.read(OWNER, handle, 12, 10).unwrap(), Vec::new());
        assert_eq!(fs.read(OWNER, handle, 999, 10).unwrap(), Vec::new());
    }

    #[test]
    fn open_rom_with_create_is_already_exists() {
        let mut fs = fresh_store();
        fs.attach_rom(&fabricated_rom()).unwrap();
        assert_eq!(
            fs.open(OWNER, wire::MODE_CREATE, "alpha.bin"),
            Err(VfsError::AlreadyExists)
        );
    }

    #[test]
    fn write_to_rom_file_is_permitted_in_write_mode() {
        let mut fs = fresh_store();
        fs.attach_rom(&fabricated_rom()).unwrap();
        let handle = fs.open(OWNER, wire::MODE_WRITE, "alpha.bin").unwrap();
        assert_eq!(fs.write(OWNER, handle, 0, b"x"), Ok(1));
        assert_eq!(fs.write(OWNER, handle, 0, &[]), Ok(0));
        assert_eq!(fs.write(OWNER, handle, 99, &[]), Ok(0));
        assert_eq!(contents(&mut fs, OWNER, handle), b"xlpha".to_vec());
    }

    #[test]
    fn rom_partial_overwrite_reads_back_after_reopen() {
        let mut fs = fresh_store();
        fs.attach_rom(&fabricated_rom()).unwrap();
        let handle = fs.open(OWNER, wire::MODE_WRITE, "beta.bin").unwrap();
        assert_eq!(fs.write(OWNER, handle, 5, b"ROM"), Ok(3));
        fs.close(OWNER, handle).unwrap();
        let reopened = fs.open(OWNER, 0, "beta.bin").unwrap();
        assert_eq!(contents(&mut fs, OWNER, reopened), b"beta-ROMload".to_vec());
        assert_eq!(fs.stat("beta.bin").unwrap(), (EntityKind::File, 12));
    }

    #[test]
    fn delete_rom_file_is_permitted() {
        let mut fs = fresh_store();
        fs.attach_rom(&fabricated_rom()).unwrap();
        assert_eq!(fs.delete("alpha.bin"), Ok(()));
        assert_eq!(fs.list().unwrap(), b"beta.bin".to_vec());
    }

    #[test]
    fn rom_delete_then_recreate_under_same_name() {
        let mut fs = fresh_store();
        fs.attach_rom(&fabricated_rom()).unwrap();
        fs.delete("alpha.bin").unwrap();
        let handle = create(&mut fs, OWNER, "alpha.bin");
        fs.write(OWNER, handle, 0, b"replaced").unwrap();
        fs.close(OWNER, handle).unwrap();
        let reopened = fs.open(OWNER, 0, "alpha.bin").unwrap();
        assert_eq!(contents(&mut fs, OWNER, reopened), b"replaced".to_vec());
        assert_eq!(fs.list().unwrap(), b"alpha.bin\nbeta.bin".to_vec());
    }

    #[test]
    fn second_attach_rom_is_already_attached() {
        let mut fs = fresh_store();
        fs.attach_rom(&fabricated_rom()).unwrap();
        assert!(matches!(
            fs.attach_rom(&fabricated_rom()),
            Err(RomMountError::AlreadyAttached)
        ));
    }

    #[test]
    fn attach_rom_rejects_format_invalid_image() {
        let mut fs = fresh_store();
        assert!(matches!(
            fs.attach_rom(b"not a rom image at all"),
            Err(RomMountError::Format(RomError::BadMagic))
        ));
        assert_eq!(fs.list().unwrap(), Vec::new());
    }

    #[test]
    fn attach_rom_rejects_path_invalid_for_vfs() {
        let mut fs = fresh_store();
        let image = system::rom::encode(&[("bad\nname", b"x".as_slice())]);
        assert!(matches!(
            fs.attach_rom(&image),
            Err(RomMountError::InvalidPath(ref name)) if name == "bad\nname"
        ));
        assert_eq!(fs.list().unwrap(), Vec::new());
    }

    #[test]
    fn attach_rom_rejects_collision_and_stores_nothing() {
        let mut fs = fresh_store();
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

    fn nested_fabricated_rom() -> Vec<u8> {
        system::rom::encode(&[
            ("games/snake", b"snake-image".as_slice()),
            ("games/deep/tetris", b"tetris-image".as_slice()),
            ("services/randomd", b"randomd-image".as_slice()),
        ])
    }

    #[test]
    fn attach_rom_lands_entries_at_nested_paths_with_implicit_parents() {
        let mut fs = fresh_store();
        fs.attach_rom(&nested_fabricated_rom()).unwrap();
        assert_eq!(fs.stat("games/snake").unwrap(), (EntityKind::File, 11));
        assert_eq!(fs.stat("games/deep/tetris").unwrap(), (EntityKind::File, 12));
        assert_eq!(fs.stat("services/randomd").unwrap(), (EntityKind::File, 13));
        assert_eq!(fs.stat("games").unwrap(), (EntityKind::Directory, 4096));
        assert_eq!(fs.stat("games/deep").unwrap(), (EntityKind::Directory, 4096));
        assert_eq!(fs.stat("services").unwrap(), (EntityKind::Directory, 4096));
        assert_eq!(fs.list().unwrap(), b"games\nservices".to_vec());
        assert_eq!(fs.list_dir("games").unwrap(), b"deep\nsnake".to_vec());
        assert_eq!(fs.list_dir("games/deep").unwrap(), b"tetris".to_vec());
    }

    #[test]
    fn attach_rom_reads_nested_entry_bytes_and_is_writable() {
        let mut fs = fresh_store();
        fs.attach_rom(&nested_fabricated_rom()).unwrap();
        let handle = fs.open(OWNER, 0, "games/snake").unwrap();
        assert_eq!(contents(&mut fs, OWNER, handle), b"snake-image".to_vec());
        fs.close(OWNER, handle).unwrap();
        let writable = fs.open(OWNER, wire::MODE_WRITE, "games/snake").unwrap();
        assert_eq!(fs.write(OWNER, writable, 0, b"SNAKE"), Ok(5));
        fs.close(OWNER, writable).unwrap();
        let reopened = fs.open(OWNER, 0, "games/snake").unwrap();
        assert_eq!(contents(&mut fs, OWNER, reopened), b"SNAKE-image".to_vec());
    }

    #[test]
    fn attach_rom_rejects_entry_under_existing_file_and_stages_nothing() {
        let mut fs = fresh_store();
        let handle = create(&mut fs, OWNER, "games");
        let image = system::rom::encode(&[
            ("services/randomd", b"r".as_slice()),
            ("games/snake", b"s".as_slice()),
        ]);
        assert!(matches!(
            fs.attach_rom(&image),
            Err(RomMountError::Duplicate(ref name)) if name == "games"
        ));
        assert_eq!(fs.stat("services"), Err(VfsError::NotFound));
        assert_eq!(fs.stat("services/randomd"), Err(VfsError::NotFound));
        fs.write(OWNER, handle, 0, b"still a file")
            .expect("collision rejection must leave the colliding file intact");
    }

    #[test]
    fn attach_rom_rejects_file_entry_conflicting_with_sibling_directory() {
        let mut fs = fresh_store();
        let image = system::rom::encode(&[
            ("services/randomd", b"r".as_slice()),
            ("services", b"s".as_slice()),
        ]);
        assert!(matches!(
            fs.attach_rom(&image),
            Err(RomMountError::Duplicate(ref name)) if name == "services"
        ));
        assert_eq!(fs.list().unwrap(), Vec::new());
    }

    fn fat_fabricated_rom() -> Vec<u8> {
        let first: Vec<u8> = (0..5000u32).map(|i| (i % 253) as u8).collect();
        let second: Vec<u8> = (0..4097u32).map(|i| (i % 197) as u8).collect();
        system::rom::encode(&[("alpha.bin", first.as_slice()), ("beta.bin", second.as_slice())])
    }

    #[test]
    fn rom_ingest_is_byte_exact_across_blocks() {
        let mut fs = fresh_store();
        fs.attach_rom(&fat_fabricated_rom()).unwrap();
        let alpha = fs.open(OWNER, 0, "alpha.bin").unwrap();
        let expected_alpha: Vec<u8> = (0..5000u32).map(|i| (i % 253) as u8).collect();
        assert_eq!(contents(&mut fs, OWNER, alpha), expected_alpha);
        let beta = fs.open(OWNER, 0, "beta.bin").unwrap();
        let expected_beta: Vec<u8> = (0..4097u32).map(|i| (i % 197) as u8).collect();
        assert_eq!(contents(&mut fs, OWNER, beta), expected_beta);
        assert_eq!(fs.list().unwrap(), b"alpha.bin\nbeta.bin".to_vec());
    }

    #[test]
    fn stat_of_created_directory_is_directory() {
        let mut fs = fresh_store();
        assert_eq!(fs.stat("hdd"), Err(VfsError::NotFound));
        fs.mkdir("hdd").unwrap();
        assert_eq!(fs.stat("hdd").unwrap(), (EntityKind::Directory, 4096));
    }

    #[test]
    fn directory_handles_answer_is_a_directory() {
        let mut fs = fresh_store();
        fs.mkdir("disk").unwrap();
        let handle = fs.open(OWNER, 0, "disk").expect("directories open like any entry");
        assert_eq!(fs.read(OWNER, handle, 0, 10), Err(VfsError::IsADirectory));
    }

    #[test]
    fn delete_and_rmdir_refuse_a_live_mount_point() {
        let mut fs = fresh_store();
        mount_fake(&mut fs, "hdd");
        assert_eq!(fs.delete("hdd"), Err(VfsError::NotPermitted));
        assert_eq!(fs.rmdir("hdd"), Err(VfsError::NotPermitted));
        fs.mounts.clear();
        fs.rmdir("hdd").expect("an unmounted plain directory removes fine");
    }

    #[test]
    fn routed_files_live_only_on_their_volume() {
        let mut fs = fresh_store();
        mount_fake(&mut fs, "disk");
        let handle = create(&mut fs, OWNER, "disk/notes.txt");
        fs.write(OWNER, handle, 0, b"on disk").unwrap();
        fs.close(OWNER, handle).unwrap();
        let read = fs.open(OWNER, 0, "disk/notes.txt").unwrap();
        assert_eq!(contents(&mut fs, OWNER, read), b"on disk");
        assert_eq!(fs.stat("notes.txt"), Err(VfsError::NotFound));
        assert_eq!(fs.stat("disk/notes.txt").unwrap(), (EntityKind::File, 7));
    }

    #[test]
    fn routed_list_dir_reads_the_volume_and_root_lists_the_mount_name() {
        let mut fs = fresh_store();
        mount_fake(&mut fs, "disk");
        create(&mut fs, OWNER, "disk/beta");
        create(&mut fs, OWNER, "disk/alpha");
        assert_eq!(fs.list_dir("disk").unwrap(), b"alpha\nbeta".to_vec());
        assert_eq!(fs.list().unwrap(), b"disk".to_vec());
    }

    #[test]
    fn unknown_prefix_is_not_found_from_the_engine_not_the_router() {
        let mut fs = fresh_store();
        assert_eq!(fs.open(OWNER, 0, "floppy/x"), Err(VfsError::NotFound));
        assert_eq!(fs.delete("floppy/x"), Err(VfsError::NotFound));
        assert_eq!(fs.stat("floppy/x"), Err(VfsError::NotFound));
        assert_eq!(fs.list_dir("floppy/x"), Err(VfsError::NotFound));
        assert_eq!(fs.mkdir("floppy/x"), Err(VfsError::NotFound));
        fs.mkdir("floppy").unwrap();
        assert_eq!(fs.mkdir("floppy/x"), Ok(()));
    }

    #[test]
    fn nested_paths_reach_the_root_engine() {
        let mut fs = fresh_store();
        fs.mkdir("games").unwrap();
        fs.mkdir("games/deep").unwrap();
        let handle = create(&mut fs, OWNER, "games/file");
        fs.write(OWNER, handle, 0, b"nested").unwrap();
        fs.close(OWNER, handle).unwrap();
        assert_eq!(fs.list_dir("games").unwrap(), b"deep\nfile".to_vec());
        let read = fs.open(OWNER, 0, "games/file").unwrap();
        assert_eq!(contents(&mut fs, OWNER, read), b"nested".to_vec());
        assert_eq!(fs.stat("games/file").unwrap(), (EntityKind::File, 6));
        fs.delete("games/file").unwrap();
        fs.rmdir("games/deep").unwrap();
        assert_eq!(fs.stat("games/deep"), Err(VfsError::NotFound));
        fs.rmdir("games").unwrap();
        assert_eq!(fs.stat("games"), Err(VfsError::NotFound));
    }

    #[test]
    fn mounted_name_shadows_its_root_directory() {
        let mut fs = fresh_store();
        mount_fake(&mut fs, "disk");
        let handle = create(&mut fs, OWNER, "disk/inner");
        fs.close(OWNER, handle).unwrap();
        assert_eq!(fs.list_dir("disk").unwrap(), b"inner".to_vec());
        let root_entries = fs.engine.readdir(b"/disk").unwrap();
        assert!(root_entries.iter().all(|item| dot_entry(&item.name)));
    }

    #[test]
    fn routed_delete_removes_volume_files() {
        let mut fs = fresh_store();
        mount_fake(&mut fs, "disk");
        let handle = create(&mut fs, OWNER, "disk/gone");
        fs.close(OWNER, handle).unwrap();
        fs.delete("disk/gone").unwrap();
        assert_eq!(fs.stat("disk/gone"), Err(VfsError::NotFound));
        assert_eq!(fs.list_dir("disk").unwrap(), Vec::new());
    }

    #[test]
    fn directories_nest_inside_a_mounted_volume() {
        let mut fs = fresh_store();
        mount_fake(&mut fs, "disk");
        fs.mkdir("disk/sub").unwrap();
        let handle = create(&mut fs, OWNER, "disk/sub/deep");
        fs.close(OWNER, handle).unwrap();
        assert_eq!(fs.list_dir("disk/sub").unwrap(), b"deep".to_vec());
        assert_eq!(fs.rmdir("disk/sub"), Err(VfsError::DirectoryNotEmpty));
        fs.delete("disk/sub/deep").unwrap();
        fs.rmdir("disk/sub").unwrap();
        assert_eq!(fs.stat("disk/sub"), Err(VfsError::NotFound));
    }

    #[test]
    fn list_dir_of_unmounted_directory_is_its_own_empty_content() {
        let mut fs = fresh_store();
        fs.mkdir("hdd").unwrap();
        assert_eq!(fs.list_dir("hdd").unwrap(), Vec::new());
    }

    #[test]
    fn list_dir_of_unknown_directory_is_not_found() {
        let mut fs = fresh_store();
        assert_eq!(fs.list_dir("floppy"), Err(VfsError::NotFound));
    }

    #[test]
    fn list_dir_of_unknown_dir_is_not_found() {
        let mut fs = fresh_store();
        assert_eq!(fs.list_dir("floppy"), Err(VfsError::NotFound));
    }

    #[test]
    fn mount_drive_of_unknown_dir_is_not_found_before_touching_the_service() {
        let mut fs = ActiveStore::new();
        assert_eq!(fs.mount_drive(1, "floppy"), Err(VfsError::NotFound));
    }

    #[test]
    fn fresh_volume_is_seeded_with_the_organization_directories() {
        let mut fs = ActiveStore::new();
        assert_eq!(fs.list().unwrap(), b"bench\ngames\nservices\ntests".to_vec());
        for seed in SEEDED_DIRECTORIES {
            assert_eq!(fs.stat(seed).unwrap(), (EntityKind::Directory, 4096));
        }
    }

    #[test]
    fn mkdir_creates_a_real_directory_in_the_root_volume() {
        let mut fs = fresh_store();
        fs.mkdir("disk").unwrap();
        assert_eq!(fs.stat("disk").unwrap(), (EntityKind::Directory, 4096));
        assert_eq!(fs.list().unwrap(), b"disk".to_vec());
        assert_eq!(fs.mkdir("disk"), Err(VfsError::AlreadyExists));
    }

    #[test]
    fn rmdir_removes_an_empty_directory() {
        let mut fs = fresh_store();
        fs.mkdir("disk").unwrap();
        fs.rmdir("disk").unwrap();
        assert_eq!(fs.stat("disk"), Err(VfsError::NotFound));
    }

    #[test]
    fn mkdir_over_a_file_collides() {
        let mut fs = fresh_store();
        create(&mut fs, OWNER, "a.txt");
        assert_eq!(fs.mkdir("a.txt"), Err(VfsError::AlreadyExists));
    }

    #[test]
    fn rmdir_on_a_file_is_not_a_directory() {
        let mut fs = fresh_store();
        create(&mut fs, OWNER, "a.txt");
        assert_eq!(fs.rmdir("a.txt"), Err(VfsError::NotDirectory));
    }

    #[test]
    fn rmdir_refuses_a_directory_holding_files() {
        let mut fs = fresh_store();
        fs.mkdir("disk").unwrap();
        let handle = fs.engine.create(b"/disk/x").unwrap();
        fs.engine.close(handle).unwrap();
        assert_eq!(fs.rmdir("disk"), Err(VfsError::DirectoryNotEmpty));
    }
}
