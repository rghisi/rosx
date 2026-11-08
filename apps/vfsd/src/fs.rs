use crate::blk::RemoteWindow;
use crate::error_map;
use alloc::collections::{BTreeMap, BTreeSet};
use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use block_protocol::manager::client::{Geometry, Manager};
use fs_core::device::BlockDevice;
use fs_core::engine::{DirItem, Engine, Kind, MetadataView};
use fs_core::error::FsError;
use rxfat::boot::BootSector;
use rxfat::{FatHandle, FatMetadata};
use rxfs::device::MemoryDevice;
use rxfs::spec::{BLOCK_BYTES, DEFAULT_BYTES_PER_INODE, FS_MAGIC, SUPERBLOCK_BLOCK};
use rxfs::superblock::UUID_BYTES;
use rxfs::volume::{FileSystem, FormatParams, Handle as RxfsHandle, Stat};
use system::rom::{RomError, RomImage};
use vfs_protocol::wire::{self, EntityKind, VfsError};

const VOLUME_BLOCKS: u64 = 2048;
const VOLUME_UUID: [u8; UUID_BYTES] = *b"rosx-ram-vol-001";
const VOLUME_NAME: &[u8] = b"rosx-ram";
const DEVICE_VOLUME_UUID: [u8; UUID_BYTES] = *b"rosx-ata-vol-001";
const DEVICE_VOLUME_NAME: &[u8] = b"rosx-hdd";
const BOOT_SECTOR_BYTES: u64 = 512;

pub type ActiveVolume = FileSystem<MemoryDevice>;
pub type ActiveStore = VfsStore<ActiveVolume, RemoteWindow>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VolumeKind {
    Rxfs,
    Fat { unit_bytes: u64 },
}

fn probe_volume(
    client: &Manager,
    device: &str,
    geometry: &Geometry,
) -> Result<VolumeKind, VfsError> {
    let device_block_bytes = u64::from(geometry.block_size);
    if device_block_bytes == 0 {
        return Err(VfsError::Unsupported);
    }
    let superblock_offset = SUPERBLOCK_BLOCK as usize * BLOCK_BYTES;
    let head_blocks = (superblock_offset + FS_MAGIC.len())
        .max(BOOT_SECTOR_BYTES as usize)
        .div_ceil(device_block_bytes as usize) as usize;
    let head = client.read_blocks(device, 0, head_blocks).map_err(error_map::block)?;
    let superblock_region = superblock_offset..superblock_offset + FS_MAGIC.len();
    if head.get(superblock_region) == Some(FS_MAGIC.as_slice()) {
        return Ok(VolumeKind::Rxfs);
    }
    let Some(sector0) = head.get(..BOOT_SECTOR_BYTES as usize) else {
        return Err(VfsError::NotFormatted);
    };
    let Some(total_sectors) = geometry
        .block_count
        .checked_mul(device_block_bytes)
        .and_then(|bytes| bytes.checked_div(BOOT_SECTOR_BYTES))
    else {
        return Err(VfsError::Unsupported);
    };
    match BootSector::parse(sector0, total_sectors) {
        Ok(boot) => Ok(VolumeKind::Fat {
            unit_bytes: u64::from(boot.bytes_per_sector),
        }),
        Err(_) => Err(VfsError::NotFormatted),
    }
}

struct MountedVolume<D: BlockDevice> {
    name: String,
    device: String,
    fs: MountedEngine<D>,
}

enum MountedEngine<D: BlockDevice> {
    Rxfs(FileSystem<D>),
    Fat(rxfat::FileSystem<RemoteWindow>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MountHandle {
    Rxfs(RxfsHandle),
    Fat(FatHandle),
}

enum MountMetadata {
    Rxfs(Stat),
    Fat(FatMetadata),
}

impl MetadataView for MountMetadata {
    fn kind(&self) -> Kind {
        match self {
            MountMetadata::Rxfs(stat) => stat.kind(),
            MountMetadata::Fat(stat) => stat.kind(),
        }
    }

    fn size(&self) -> u64 {
        match self {
            MountMetadata::Rxfs(stat) => stat.size(),
            MountMetadata::Fat(stat) => stat.size(),
        }
    }
}

impl<D: BlockDevice> Engine for MountedEngine<D> {
    type Handle = MountHandle;
    type Metadata = MountMetadata;

    fn lookup(&mut self, path_bytes: &[u8]) -> Result<MountMetadata, FsError> {
        match self {
            MountedEngine::Rxfs(fs) => fs.lookup(path_bytes).map(MountMetadata::Rxfs),
            MountedEngine::Fat(fs) => fs.lookup(path_bytes).map(MountMetadata::Fat),
        }
    }

    fn open(&mut self, path_bytes: &[u8]) -> Result<MountHandle, FsError> {
        match self {
            MountedEngine::Rxfs(fs) => fs.open(path_bytes).map(MountHandle::Rxfs),
            MountedEngine::Fat(fs) => fs.open(path_bytes).map(MountHandle::Fat),
        }
    }

    fn create(&mut self, path_bytes: &[u8]) -> Result<MountHandle, FsError> {
        match self {
            MountedEngine::Rxfs(fs) => fs.create(path_bytes).map(MountHandle::Rxfs),
            MountedEngine::Fat(fs) => fs.create(path_bytes).map(MountHandle::Fat),
        }
    }

    fn mkdir(&mut self, path_bytes: &[u8]) -> Result<(), FsError> {
        match self {
            MountedEngine::Rxfs(fs) => fs.mkdir(path_bytes),
            MountedEngine::Fat(fs) => fs.mkdir(path_bytes),
        }
    }

    fn rmdir(&mut self, path_bytes: &[u8]) -> Result<(), FsError> {
        match self {
            MountedEngine::Rxfs(fs) => fs.rmdir(path_bytes),
            MountedEngine::Fat(fs) => fs.rmdir(path_bytes),
        }
    }

    fn read(&mut self, handle: MountHandle, offset: u64, buf: &mut [u8]) -> Result<usize, FsError> {
        match (self, handle) {
            (MountedEngine::Rxfs(fs), MountHandle::Rxfs(engine_handle)) => {
                fs.read(engine_handle, offset, buf)
            }
            (MountedEngine::Fat(fs), MountHandle::Fat(engine_handle)) => {
                fs.read(engine_handle, offset, buf)
            }
            _ => Err(FsError::BadHandle),
        }
    }

    fn write(
        &mut self,
        handle: MountHandle,
        offset: u64,
        data: &[u8],
    ) -> Result<usize, FsError> {
        match (self, handle) {
            (MountedEngine::Rxfs(fs), MountHandle::Rxfs(engine_handle)) => {
                fs.write(engine_handle, offset, data)
            }
            (MountedEngine::Fat(fs), MountHandle::Fat(engine_handle)) => {
                fs.write(engine_handle, offset, data)
            }
            _ => Err(FsError::BadHandle),
        }
    }

    fn stat(&mut self, handle: MountHandle) -> Result<MountMetadata, FsError> {
        match (self, handle) {
            (MountedEngine::Rxfs(fs), MountHandle::Rxfs(engine_handle)) => {
                fs.stat(engine_handle).map(MountMetadata::Rxfs)
            }
            (MountedEngine::Fat(fs), MountHandle::Fat(engine_handle)) => {
                fs.stat(engine_handle).map(MountMetadata::Fat)
            }
            _ => Err(FsError::BadHandle),
        }
    }

    fn close(&mut self, handle: MountHandle) -> Result<(), FsError> {
        match (self, handle) {
            (MountedEngine::Rxfs(fs), MountHandle::Rxfs(engine_handle)) => fs.close(engine_handle),
            (MountedEngine::Fat(fs), MountHandle::Fat(engine_handle)) => fs.close(engine_handle),
            _ => Err(FsError::BadHandle),
        }
    }

    fn remove(&mut self, path_bytes: &[u8]) -> Result<(), FsError> {
        match self {
            MountedEngine::Rxfs(fs) => fs.remove(path_bytes),
            MountedEngine::Fat(fs) => fs.remove(path_bytes),
        }
    }

    fn readdir(&mut self, path_bytes: &[u8]) -> Result<Vec<DirItem>, FsError> {
        match self {
            MountedEngine::Rxfs(fs) => fs.readdir(path_bytes),
            MountedEngine::Fat(fs) => fs.readdir(path_bytes),
        }
    }

    fn sync(&mut self) -> Result<(), FsError> {
        match self {
            MountedEngine::Rxfs(fs) => fs.sync(),
            MountedEngine::Fat(fs) => fs.sync(),
        }
    }
}

enum OpenLocation<E: Engine> {
    Root(E::Handle),
    Mount { slot: usize, handle: MountHandle },
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
    name: String,
    location: OpenLocation<E>,
}

#[derive(Debug)]
pub enum RomMountError {
    Format(RomError),
    InvalidName(String),
    Duplicate(String),
    AlreadyAttached,
    Engine(FsError),
}

pub struct VfsStore<E: Engine, D: BlockDevice> {
    engine: E,
    mounts: Vec<MountedVolume<D>>,
    opens: BTreeMap<u32, OpenEntry<E>>,
    next_handle: u32,
    rom_names: BTreeSet<String>,
    rom_attached: bool,
}

fn dot_entry(name: &[u8]) -> bool {
    name == b".".as_slice() || name == b"..".as_slice()
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

fn readdir_blob<E2: Engine>(engine: &mut E2, path: &[u8]) -> Result<Vec<u8>, VfsError> {
    let items = engine.readdir(path).map_err(error_map::fs)?;
    let mut names: Vec<&[u8]> = items
        .iter()
        .map(|item| item.name.as_slice())
        .filter(|name| !dot_entry(name))
        .collect();
    names.sort_unstable();
    names_blob(&names)
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
        Ok(_) => engine.open(path).map_err(error_map::fs),
        Err(FsError::NotFound) if mode & wire::MODE_CREATE != 0 => {
            let handle = engine.create(path).map_err(error_map::fs)?;
            engine.sync().map_err(error_map::fs)?;
            Ok(handle)
        }
        Err(FsError::NotFound) => Err(VfsError::NotFound),
        Err(error) => Err(error_map::fs(error)),
    }
}

impl VfsStore<ActiveVolume, RemoteWindow> {
    pub fn new() -> VfsStore<ActiveVolume, RemoteWindow> {
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

    fn open_window(&self, device: &str, unit_bytes: u64) -> Result<RemoteWindow, VfsError> {
        let client = Manager::connect().map_err(error_map::block)?;
        let geometry = match client.probe(device) {
            Ok(geometry) => geometry,
            Err(error) => {
                client.disconnect();
                return Err(error_map::block(error));
            }
        };
        match RemoteWindow::new(client, device, &geometry, unit_bytes) {
            Ok(disk) => Ok(disk),
            Err(client) => {
                client.disconnect();
                Err(VfsError::Unsupported)
            }
        }
    }

    pub fn format_device(&mut self, device: &str) -> Result<(), VfsError> {
        if self.device_busy(device) {
            return Err(VfsError::Busy);
        }
        let disk = self.open_window(device, BLOCK_BYTES as u64)?;
        let params = FormatParams {
            bytes_per_inode: DEFAULT_BYTES_PER_INODE,
            uuid: DEVICE_VOLUME_UUID,
            volume_name: DEVICE_VOLUME_NAME,
            created_at: 0,
        };
        let fs = FileSystem::format(disk, &params).map_err(error_map::fs)?;
        fs.unmount()
            .map(|disk| disk.into_client().disconnect())
            .map_err(error_map::fs)
    }

    pub fn mount_device(&mut self, device: &str, dir: &str) -> Result<(), VfsError> {
        if !wire::valid_mount_dir(dir) {
            return Err(VfsError::InvalidPath);
        }
        let name = &dir[1..];
        if self.mounts.iter().any(|mount| mount.device == device)
            || self.mounts.iter().any(|mount| mount.name == name)
        {
            return Err(VfsError::Busy);
        }
        let mount_point = self.engine.lookup(dir.as_bytes()).map_err(error_map::fs)?;
        if mount_point.kind() != Kind::Directory {
            return Err(VfsError::NotDirectory);
        }
        let client = Manager::connect().map_err(error_map::block)?;
        let geometry = match client.probe(device) {
            Ok(geometry) => geometry,
            Err(error) => {
                client.disconnect();
                return Err(error_map::block(error));
            }
        };
        let kind = match probe_volume(&client, device, &geometry) {
            Ok(kind) => kind,
            Err(error) => {
                client.disconnect();
                return Err(error);
            }
        };
        let unit_bytes = match kind {
            VolumeKind::Rxfs => BLOCK_BYTES as u64,
            VolumeKind::Fat { unit_bytes } => unit_bytes,
        };
        let disk = match RemoteWindow::new(client, device, &geometry, unit_bytes) {
            Ok(disk) => disk,
            Err(client) => {
                client.disconnect();
                return Err(VfsError::Unsupported);
            }
        };
        let fs = match kind {
            VolumeKind::Rxfs => MountedEngine::Rxfs(FileSystem::mount(disk).map_err(error_map::mount)?),
            VolumeKind::Fat { .. } => {
                MountedEngine::Fat(rxfat::FileSystem::new(disk).map_err(error_map::mount)?)
            }
        };
        self.mounts.push(MountedVolume {
            name: String::from(name),
            device: String::from(device),
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
            rom_names: BTreeSet::new(),
            rom_attached: false,
        }
    }

    fn route(&self, path: &str) -> Result<Target, VfsError> {
        let body = path.strip_prefix('/').unwrap_or(path);
        let (head, rest) = match body.split_once('/') {
            Some(split) => split,
            None => (body, ""),
        };
        match self.mounts.iter().position(|mount| mount.name == head) {
            Some(slot) => Ok(Target::Volume(slot, format!("/{rest}"))),
            None if rest.is_empty() => Ok(Target::Root),
            None => Err(VfsError::NotFound),
        }
    }

    fn volume_mut(&mut self, slot: usize) -> Result<&mut MountedEngine<D>, VfsError> {
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

    fn is_mount_point(&self, path: &str) -> bool {
        let Some(body) = path.strip_prefix('/') else {
            return false;
        };
        let (head, rest) = match body.split_once('/') {
            Some(split) => split,
            None => (body, ""),
        };
        rest.is_empty() && self.mounts.iter().any(|mount| mount.name == head)
    }

    pub fn device_busy(&self, id: &str) -> bool {
        self.mounts.iter().any(|mount| mount.device == id)
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
            if self.engine.lookup(format!("/{}", entry.name).as_bytes()).is_ok() {
                return Err(RomMountError::Duplicate(String::from(entry.name)));
            }
            staged.push((entry.name, entry.offset, entry.len));
        }
        for (name, offset, len) in staged {
            let handle = self
                .engine
                .create(format!("/{name}").as_bytes())
                .map_err(RomMountError::Engine)?;
            if len > 0 {
                self.engine
                    .write(handle, 0, &image[offset..offset + len])
                    .map_err(RomMountError::Engine)?;
            }
            self.engine.close(handle).map_err(RomMountError::Engine)?;
            self.rom_names.insert(format!("/{name}"));
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
                let handle = open_engine(&mut self.engine, mode, name.as_bytes())?;
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
                name: String::from(name),
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
        .map_err(error_map::fs)?;
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
        let location = entry.location;
        let written = match location {
            OpenLocation::Root(engine_handle) => {
                let written = self
                    .engine
                    .write(engine_handle, offset, payload)
                    .map_err(error_map::fs)?;
                self.engine.sync().map_err(error_map::fs)?;
                written
            }
            OpenLocation::Mount { slot, handle } => {
                let volume = self.volume_mut(slot)?;
                let written = volume.write(handle, offset, payload).map_err(error_map::fs)?;
                volume.sync().map_err(error_map::fs)?;
                written
            }
        };
        Ok(written)
    }

    pub fn close(&mut self, owner: u32, handle: u32) -> Result<(), VfsError> {
        let location = self.live_location(owner, handle)?;
        match location {
            OpenLocation::Root(engine_handle) => {
                self.engine.close(engine_handle).map_err(error_map::fs)?
            }
            OpenLocation::Mount { slot, handle } => {
                self.volume_mut(slot)?.close(handle).map_err(error_map::fs)?
            }
        }
        self.opens.remove(&handle);
        Ok(())
    }

    pub fn delete(&mut self, name: &str) -> Result<(), VfsError> {
        if !wire::valid_path(name) {
            return Err(VfsError::InvalidPath);
        }
        if name == "/" || self.is_mount_point(name) {
            return Err(VfsError::NotPermitted);
        }
        match self.route(name)? {
            Target::Root => {
                if self.rom_names.contains(name) {
                    return Err(VfsError::NotPermitted);
                }
                self.engine.remove(name.as_bytes()).map_err(error_map::fs)?;
                self.engine.sync().map_err(error_map::fs)
            }
            Target::Volume(slot, path) => {
                let volume = self.volume_mut(slot)?;
                volume.remove(path.as_bytes()).map_err(error_map::fs)?;
                volume.sync().map_err(error_map::fs)
            }
        }
    }

    pub fn mkdir(&mut self, path: &str) -> Result<(), VfsError> {
        if !wire::valid_path(path) {
            return Err(VfsError::InvalidPath);
        }
        match self.route(path)? {
            Target::Root => {
                self.engine.mkdir(path.as_bytes()).map_err(error_map::fs)?;
                self.engine.sync().map_err(error_map::fs)
            }
            Target::Volume(slot, path) => {
                let volume = self.volume_mut(slot)?;
                volume.mkdir(path.as_bytes()).map_err(error_map::fs)?;
                volume.sync().map_err(error_map::fs)
            }
        }
    }

    pub fn rmdir(&mut self, path: &str) -> Result<(), VfsError> {
        if !wire::valid_path(path) {
            return Err(VfsError::InvalidPath);
        }
        if path == "/" || self.is_mount_point(path) {
            return Err(VfsError::NotPermitted);
        }
        match self.route(path)? {
            Target::Root => {
                self.engine.rmdir(path.as_bytes()).map_err(error_map::fs)?;
                self.engine.sync().map_err(error_map::fs)
            }
            Target::Volume(slot, path) => {
                let volume = self.volume_mut(slot)?;
                volume.rmdir(path.as_bytes()).map_err(error_map::fs)?;
                volume.sync().map_err(error_map::fs)
            }
        }
    }

    pub fn stat(&mut self, name: &str) -> Result<(EntityKind, u64), VfsError> {
        if !wire::valid_path(name) {
            return Err(VfsError::InvalidPath);
        }
        let (kind, size) = match self.route(name)? {
            Target::Root => {
                let stat = self.engine.lookup(name.as_bytes()).map_err(error_map::fs)?;
                (stat.kind(), stat.size())
            }
            Target::Volume(slot, path) => {
                let stat = self
                    .volume_mut(slot)?
                    .lookup(path.as_bytes())
                    .map_err(error_map::fs)?;
                (stat.kind(), stat.size())
            }
        };
        let kind = match kind {
            Kind::Regular => EntityKind::File,
            Kind::Directory => EntityKind::Directory,
        };
        Ok((kind, size))
    }

    pub fn list(&mut self, path: &str) -> Result<Vec<u8>, VfsError> {
        if !wire::valid_path(path) {
            return Err(VfsError::InvalidPath);
        }
        match self.route(path)? {
            Target::Root => readdir_blob(&mut self.engine, path.as_bytes()),
            Target::Volume(slot, path) => {
                let volume = self.volume_mut(slot)?;
                readdir_blob(volume, path.as_bytes())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OWNER: u32 = 0x0010_0001;
    const INTRUDER: u32 = 0x0020_0002;

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
        fs.mkdir(&format!("/{name}")).unwrap();
        let params = FormatParams {
            bytes_per_inode: DEFAULT_BYTES_PER_INODE,
            uuid: DEVICE_VOLUME_UUID,
            volume_name: DEVICE_VOLUME_NAME,
            created_at: 0,
        };
        let volume = FileSystem::format(MemoryDevice::new(64), &params).unwrap();
        fs.mounts.push(MountedVolume {
            name: String::from(name),
            device: String::from(name),
            fs: MountedEngine::Rxfs(volume),
        });
    }

    fn create<E: Engine, D: BlockDevice>(fs: &mut VfsStore<E, D>, owner: u32, path: &str) -> u32 {
        fs.open(owner, wire::MODE_CREATE | wire::MODE_WRITE, path)
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
        let handle = create(&mut fs, OWNER, "/a.txt");
        assert_eq!(handle, 1);
        assert_eq!(contents(&mut fs, OWNER, handle), Vec::new());
    }

    #[test]
    fn open_existing_without_create_reuses_existing_content() {
        let mut fs = fresh_store();
        let first = create(&mut fs, OWNER, "/a.txt");
        fs.write(OWNER, first, 0, b"abc").unwrap();
        fs.close(OWNER, first).unwrap();
        let second = fs.open(OWNER, 0, "/a.txt").expect("plain open must succeed");
        assert_eq!(second, 2);
        assert_eq!(contents(&mut fs, OWNER, second), b"abc".to_vec());
    }

    #[test]
    fn open_existing_with_create_is_already_exists() {
        let mut fs = fresh_store();
        create(&mut fs, OWNER, "/a.txt");
        assert_eq!(
            fs.open(OWNER, wire::MODE_CREATE, "/a.txt"),
            Err(VfsError::AlreadyExists)
        );
    }

    #[test]
    fn open_missing_without_create_is_not_found() {
        let mut fs = fresh_store();
        assert_eq!(fs.open(OWNER, 0, "/ghost.txt"), Err(VfsError::NotFound));
    }

    #[test]
    fn handles_increase_monotonically() {
        let mut fs = fresh_store();
        assert_eq!(create(&mut fs, OWNER, "/a"), 1);
        assert_eq!(create(&mut fs, OWNER, "/b"), 2);
        assert_eq!(create(&mut fs, OWNER, "/c"), 3);
    }

    #[test]
    fn open_invalid_name_is_invalid_path() {
        let mut fs = fresh_store();
        assert_eq!(
            fs.open(OWNER, wire::MODE_CREATE, ""),
            Err(VfsError::InvalidPath)
        );
        assert_eq!(
            fs.open(OWNER, wire::MODE_CREATE, "a.txt"),
            Err(VfsError::InvalidPath)
        );
        assert_eq!(
            fs.open(
                OWNER,
                wire::MODE_CREATE,
                &format!("/{}", name_of_len(wire::MAX_NAME_BYTES)),
            ),
            Err(VfsError::InvalidPath)
        );
        assert_eq!(
            fs.open(OWNER, wire::MODE_CREATE, "/bad\nname"),
            Err(VfsError::InvalidPath)
        );
    }

    #[test]
    fn delete_invalid_name_is_invalid_path() {
        let mut fs = fresh_store();
        assert_eq!(fs.delete("/bad\nname"), Err(VfsError::InvalidPath));
        assert_eq!(fs.delete("bad\nname"), Err(VfsError::InvalidPath));
        assert_eq!(fs.delete(""), Err(VfsError::InvalidPath));
    }

    #[test]
    fn read_at_end_is_empty() {
        let mut fs = fresh_store();
        let handle = create(&mut fs, OWNER, "/a.txt");
        fs.write(OWNER, handle, 0, b"abc").unwrap();
        assert_eq!(fs.read(OWNER, handle, 3, 10).unwrap(), Vec::new());
    }

    #[test]
    fn read_past_end_is_empty() {
        let mut fs = fresh_store();
        let handle = create(&mut fs, OWNER, "/a.txt");
        fs.write(OWNER, handle, 0, b"abc").unwrap();
        assert_eq!(fs.read(OWNER, handle, 999, 10).unwrap(), Vec::new());
    }

    #[test]
    fn read_clamps_across_end() {
        let mut fs = fresh_store();
        let handle = create(&mut fs, OWNER, "/a.txt");
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
        let handle = create(&mut fs, OWNER, "/a.txt");
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
        let created = create(&mut fs, OWNER, "/a.txt");
        fs.write(OWNER, created, 0, b"abc").unwrap();
        fs.close(OWNER, created).unwrap();
        let readonly = fs.open(OWNER, 0, "/a.txt").unwrap();
        assert_eq!(
            fs.write(OWNER, readonly, 0, b"x"),
            Err(VfsError::NotPermitted)
        );
        assert_eq!(contents(&mut fs, OWNER, readonly), b"abc".to_vec());
    }

    #[test]
    fn read_is_permitted_on_mode_zero_handle() {
        let mut fs = fresh_store();
        let created = create(&mut fs, OWNER, "/a.txt");
        fs.write(OWNER, created, 0, b"hello").unwrap();
        fs.close(OWNER, created).unwrap();
        let readonly = fs.open(OWNER, 0, "/a.txt").unwrap();
        assert_eq!(contents(&mut fs, OWNER, readonly), b"hello".to_vec());
    }

    #[test]
    fn write_at_zero_into_empty_file() {
        let mut fs = fresh_store();
        let handle = create(&mut fs, OWNER, "/a.txt");
        assert_eq!(fs.write(OWNER, handle, 0, b"hi"), Ok(2));
        assert_eq!(contents(&mut fs, OWNER, handle), b"hi".to_vec());
    }

    #[test]
    fn write_appending_at_len() {
        let mut fs = fresh_store();
        let handle = create(&mut fs, OWNER, "/a.txt");
        fs.write(OWNER, handle, 0, b"ab").unwrap();
        assert_eq!(fs.write(OWNER, handle, 2, b"cd"), Ok(2));
        assert_eq!(contents(&mut fs, OWNER, handle), b"abcd".to_vec());
    }

    #[test]
    fn write_beyond_len_zero_fills_hole_byte_exactly() {
        let mut fs = fresh_store();
        let handle = create(&mut fs, OWNER, "/a.txt");
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
        let handle = create(&mut fs, OWNER, "/a.txt");
        fs.write(OWNER, handle, 0, b"abc").unwrap();
        assert_eq!(fs.write(OWNER, handle, 100, &[]), Ok(0));
        assert_eq!(contents(&mut fs, OWNER, handle), b"abc".to_vec());
    }

    #[test]
    fn close_frees_handle_and_second_close_is_bad_handle() {
        let mut fs = fresh_store();
        let handle = create(&mut fs, OWNER, "/a.txt");
        assert_eq!(fs.close(OWNER, handle), Ok(()));
        assert_eq!(fs.close(OWNER, handle), Err(VfsError::BadHandle));
    }

    #[test]
    fn delete_then_read_via_still_open_handle_is_bad_handle() {
        let mut fs = fresh_store();
        let handle = create(&mut fs, OWNER, "/a.txt");
        fs.write(OWNER, handle, 0, b"abc").unwrap();
        assert_eq!(fs.delete("/a.txt"), Ok(()));
        assert_eq!(fs.read(OWNER, handle, 0, 10), Err(VfsError::BadHandle));
        assert_eq!(
            fs.write(OWNER, handle, 0, b"x"),
            Err(VfsError::BadHandle)
        );
    }

    #[test]
    fn handle_to_deleted_file_still_closes_cleanly() {
        let mut fs = fresh_store();
        let handle = create(&mut fs, OWNER, "/a.txt");
        fs.delete("/a.txt").unwrap();
        assert_eq!(fs.close(OWNER, handle), Ok(()));
    }

    #[test]
    fn delete_missing_is_not_found() {
        let mut fs = fresh_store();
        assert_eq!(fs.delete("/ghost.txt"), Err(VfsError::NotFound));
    }

    #[test]
    fn stat_missing_is_not_found() {
        let mut fs = fresh_store();
        assert_eq!(fs.stat("/ghost.txt"), Err(VfsError::NotFound));
    }

    #[test]
    fn stat_created_empty_file_is_zero_size() {
        let mut fs = fresh_store();
        create(&mut fs, OWNER, "/a.txt");
        assert_eq!(fs.stat("/a.txt").unwrap(), (EntityKind::File, 0));
    }

    #[test]
    fn stat_tracks_writes_and_holes() {
        let mut fs = fresh_store();
        let handle = create(&mut fs, OWNER, "/a.txt");
        fs.write(OWNER, handle, 0, b"ab").unwrap();
        fs.write(OWNER, handle, 4096, b"c").unwrap();
        assert_eq!(fs.stat("/a.txt").unwrap(), (EntityKind::File, 4097));
    }

    #[test]
    fn stat_of_deleted_name_is_not_found() {
        let mut fs = fresh_store();
        create(&mut fs, OWNER, "/a.txt");
        fs.delete("/a.txt").unwrap();
        assert_eq!(fs.stat("/a.txt"), Err(VfsError::NotFound));
    }

    #[test]
    fn stat_invalid_name_is_invalid_path() {
        let mut fs = fresh_store();
        assert_eq!(fs.stat(""), Err(VfsError::InvalidPath));
        assert_eq!(fs.stat("a.txt"), Err(VfsError::InvalidPath));
        assert_eq!(fs.stat("bad//name"), Err(VfsError::InvalidPath));
        assert_eq!(fs.stat("."), Err(VfsError::InvalidPath));
        assert_eq!(fs.stat(".."), Err(VfsError::InvalidPath));
    }

    #[test]
    fn root_is_statable_but_undeletable() {
        let mut fs = fresh_store();
        assert_eq!(fs.stat("/").unwrap().0, EntityKind::Directory);
        assert_eq!(fs.delete("/"), Err(VfsError::NotPermitted));
        assert_eq!(fs.rmdir("/"), Err(VfsError::NotPermitted));
    }

    #[test]
    fn stat_succeeds_when_all_engine_slots_are_open() {
        let mut fs = fresh_store();
        create(&mut fs, OWNER, "/probe.txt");
        for index in 0..63u32 {
            create(&mut fs, OWNER, &format!("/s{index}"));
        }
        assert_eq!(
            fs.open(OWNER, wire::MODE_CREATE, "/overflow"),
            Err(VfsError::ServerResource)
        );
        assert_eq!(fs.stat("/probe.txt").unwrap(), (EntityKind::File, 0));
    }

    #[test]
    fn rom_file_stat_reports_exact_length() {
        let mut fs = fresh_store();
        fs.attach_rom(&fabricated_rom()).unwrap();
        assert_eq!(fs.stat("/alpha.bin").unwrap(), (EntityKind::File, 5));
        assert_eq!(fs.stat("/beta.bin").unwrap(), (EntityKind::File, 12));
    }

    #[test]
    fn list_returns_sorted_newline_joined_names() {
        let mut fs = fresh_store();
        create(&mut fs, OWNER, "/notes.txt");
        create(&mut fs, OWNER, "/blob.bin");
        assert_eq!(fs.list("/").unwrap(), b"blob.bin\nnotes.txt".to_vec());
    }

    #[test]
    fn list_of_empty_root_is_empty() {
        let mut fs = fresh_store();
        assert_eq!(fs.list("/").unwrap(), Vec::new());
    }

    #[test]
    fn two_handles_on_one_file_see_shared_content() {
        let mut fs = fresh_store();
        let writer = create(&mut fs, OWNER, "/a.txt");
        let reader = fs.open(OWNER, 0, "/a.txt").unwrap();
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
            held.push(create(&mut fs, OWNER, &format!("/s{index}")));
        }
        assert_eq!(
            fs.open(OWNER, wire::MODE_CREATE | wire::MODE_WRITE, "/overflow"),
            Err(VfsError::ServerResource)
        );
        assert_eq!(fs.close(OWNER, held[0]), Ok(()));
        let revived = fs
            .open(OWNER, wire::MODE_CREATE | wire::MODE_WRITE, "/spare")
            .expect("closing an open must free an engine slot");
        assert!(revived > 64);
    }

    #[test]
    fn create_beyond_inode_capacity_is_server_resource() {
        let mut fs = fresh_store();
        let mut created = 0u32;
        loop {
            let path = format!("/exhaust{created}");
            let handle = match fs.open(OWNER, wire::MODE_CREATE, &path) {
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
        let blob = fs.list("/").unwrap();
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
        create(&mut fs, OWNER, "/mid.txt");
        assert_eq!(
            fs.list("/").unwrap(),
            b"alpha.bin\nbeta.bin\nmid.txt".to_vec()
        );
    }

    #[test]
    fn rom_file_reads_exact_bytes() {
        let mut fs = fresh_store();
        fs.attach_rom(&fabricated_rom()).unwrap();
        let handle = fs.open(OWNER, 0, "/alpha.bin").unwrap();
        assert_eq!(contents(&mut fs, OWNER, handle), b"alpha".to_vec());
        assert_eq!(fs.read(OWNER, handle, 2, 2).unwrap(), b"ph".to_vec());
    }

    #[test]
    fn rom_read_clamps_across_window_end_and_eof() {
        let mut fs = fresh_store();
        fs.attach_rom(&fabricated_rom()).unwrap();
        let handle = fs.open(OWNER, 0, "/beta.bin").unwrap();
        assert_eq!(fs.read(OWNER, handle, 5, 100).unwrap(), b"payload".to_vec());
        assert_eq!(fs.read(OWNER, handle, 12, 10).unwrap(), Vec::new());
        assert_eq!(fs.read(OWNER, handle, 999, 10).unwrap(), Vec::new());
    }

    #[test]
    fn open_rom_with_create_is_already_exists() {
        let mut fs = fresh_store();
        fs.attach_rom(&fabricated_rom()).unwrap();
        assert_eq!(
            fs.open(OWNER, wire::MODE_CREATE, "/alpha.bin"),
            Err(VfsError::AlreadyExists)
        );
    }

    #[test]
    fn write_to_rom_file_is_not_permitted_even_when_empty() {
        let mut fs = fresh_store();
        fs.attach_rom(&fabricated_rom()).unwrap();
        let handle = fs.open(OWNER, wire::MODE_WRITE, "/alpha.bin").unwrap();
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
        let mut fs = fresh_store();
        fs.attach_rom(&fabricated_rom()).unwrap();
        assert_eq!(fs.delete("/alpha.bin"), Err(VfsError::NotPermitted));
        assert_eq!(fs.list("/").unwrap(), b"alpha.bin\nbeta.bin".to_vec());
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
        assert_eq!(fs.list("/").unwrap(), Vec::new());
    }

    #[test]
    fn attach_rom_rejects_name_invalid_for_vfs() {
        let mut fs = fresh_store();
        let image = system::rom::encode(&[("bad\nname", b"x".as_slice())]);
        assert!(matches!(
            fs.attach_rom(&image),
            Err(RomMountError::InvalidName(ref name)) if name == "bad\nname"
        ));
        assert_eq!(fs.list("/").unwrap(), Vec::new());
    }

    #[test]
    fn attach_rom_rejects_collision_and_stores_nothing() {
        let mut fs = fresh_store();
        let handle = create(&mut fs, OWNER, "/alpha.bin");
        assert!(matches!(
            fs.attach_rom(&fabricated_rom()),
            Err(RomMountError::Duplicate(ref name)) if name == "alpha.bin"
        ));
        assert_eq!(fs.list("/").unwrap(), b"alpha.bin".to_vec());
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
        let mut fs = fresh_store();
        fs.attach_rom(&fat_fabricated_rom()).unwrap();
        let alpha = fs.open(OWNER, 0, "/alpha.bin").unwrap();
        let expected_alpha: Vec<u8> = (0..5000u32).map(|i| (i % 253) as u8).collect();
        assert_eq!(contents(&mut fs, OWNER, alpha), expected_alpha);
        let beta = fs.open(OWNER, 0, "/beta.bin").unwrap();
        let expected_beta: Vec<u8> = (0..4097u32).map(|i| (i % 197) as u8).collect();
        assert_eq!(contents(&mut fs, OWNER, beta), expected_beta);
        assert_eq!(fs.list("/").unwrap(), b"alpha.bin\nbeta.bin".to_vec());
    }

    #[test]
    fn stat_of_created_directory_is_directory() {
        let mut fs = fresh_store();
        assert_eq!(fs.stat("/hdd"), Err(VfsError::NotFound));
        fs.mkdir("/hdd").unwrap();
        assert_eq!(fs.stat("/hdd").unwrap(), (EntityKind::Directory, 4096));
    }

    #[test]
    fn directory_handles_answer_is_a_directory() {
        let mut fs = fresh_store();
        fs.mkdir("/disk").unwrap();
        let handle = fs.open(OWNER, 0, "/disk").expect("directories open like any entry");
        assert_eq!(fs.read(OWNER, handle, 0, 10), Err(VfsError::IsADirectory));
    }

    #[test]
    fn delete_and_rmdir_refuse_a_live_mount_point() {
        let mut fs = fresh_store();
        mount_fake(&mut fs, "hdd");
        assert_eq!(fs.delete("/hdd"), Err(VfsError::NotPermitted));
        assert_eq!(fs.rmdir("/hdd"), Err(VfsError::NotPermitted));
        fs.mounts.clear();
        fs.rmdir("/hdd").expect("an unmounted plain directory removes fine");
    }

    #[test]
    fn routed_files_live_only_on_their_volume() {
        let mut fs = fresh_store();
        mount_fake(&mut fs, "disk");
        let handle = create(&mut fs, OWNER, "/disk/notes.txt");
        fs.write(OWNER, handle, 0, b"on disk").unwrap();
        fs.close(OWNER, handle).unwrap();
        let read = fs.open(OWNER, 0, "/disk/notes.txt").unwrap();
        assert_eq!(contents(&mut fs, OWNER, read), b"on disk");
        assert_eq!(fs.stat("/notes.txt"), Err(VfsError::NotFound));
        assert_eq!(fs.stat("/disk/notes.txt").unwrap(), (EntityKind::File, 7));
        assert_eq!(fs.list("/").unwrap(), b"disk".to_vec());
    }

    #[test]
    fn routed_list_reads_the_volume() {
        let mut fs = fresh_store();
        mount_fake(&mut fs, "disk");
        create(&mut fs, OWNER, "/disk/beta");
        create(&mut fs, OWNER, "/disk/alpha");
        assert_eq!(fs.list("/disk").unwrap(), b"alpha\nbeta".to_vec());
        assert_eq!(fs.list("/").unwrap(), b"disk".to_vec());
    }

    #[test]
    fn unknown_mount_prefix_is_not_found_for_every_operation() {
        let mut fs = fresh_store();
        assert_eq!(fs.open(OWNER, 0, "/floppy/x"), Err(VfsError::NotFound));
        assert_eq!(fs.delete("/floppy/x"), Err(VfsError::NotFound));
        assert_eq!(fs.stat("/floppy/x"), Err(VfsError::NotFound));
        assert_eq!(fs.list("/floppy/x"), Err(VfsError::NotFound));
        assert_eq!(fs.mkdir("/floppy/x"), Err(VfsError::NotFound));
    }

    #[test]
    fn routed_delete_removes_volume_files() {
        let mut fs = fresh_store();
        mount_fake(&mut fs, "disk");
        let handle = create(&mut fs, OWNER, "/disk/gone");
        fs.close(OWNER, handle).unwrap();
        fs.delete("/disk/gone").unwrap();
        assert_eq!(fs.stat("/disk/gone"), Err(VfsError::NotFound));
        assert_eq!(fs.list("/disk").unwrap(), Vec::new());
    }

    #[test]
    fn directories_nest_inside_a_mounted_volume() {
        let mut fs = fresh_store();
        mount_fake(&mut fs, "disk");
        fs.mkdir("/disk/sub").unwrap();
        let handle = create(&mut fs, OWNER, "/disk/sub/deep");
        fs.close(OWNER, handle).unwrap();
        assert_eq!(fs.list("/disk/sub").unwrap(), b"deep".to_vec());
        assert_eq!(fs.rmdir("/disk/sub"), Err(VfsError::DirectoryNotEmpty));
        fs.delete("/disk/sub/deep").unwrap();
        fs.rmdir("/disk/sub").unwrap();
        assert_eq!(fs.stat("/disk/sub"), Err(VfsError::NotFound));
    }

    #[test]
    fn list_of_unmounted_directory_is_its_own_empty_content() {
        let mut fs = fresh_store();
        fs.mkdir("/hdd").unwrap();
        assert_eq!(fs.list("/hdd").unwrap(), Vec::new());
    }

    #[test]
    fn list_of_unknown_directory_is_not_found() {
        let mut fs = fresh_store();
        assert_eq!(fs.list("/floppy"), Err(VfsError::NotFound));
    }

    #[test]
    fn list_of_relative_directory_is_invalid_path() {
        let mut fs = fresh_store();
        assert_eq!(fs.list("floppy"), Err(VfsError::InvalidPath));
    }

    #[test]
    fn mount_device_of_unknown_dir_is_not_found_before_touching_the_service() {
        let mut fs = ActiveStore::new();
        assert_eq!(fs.mount_device("ata0", "/floppy"), Err(VfsError::NotFound));
    }

    #[test]
    fn mount_device_of_relative_or_nested_dir_is_invalid_path() {
        let mut fs = ActiveStore::new();
        assert_eq!(fs.mount_device("ata0", "hdd"), Err(VfsError::InvalidPath));
        assert_eq!(
            fs.mount_device("ata0", "/hdd/sub"),
            Err(VfsError::InvalidPath)
        );
    }

    #[test]
    fn a_live_volume_marks_its_blockd_device_busy() {
        let mut fs = fresh_store();
        assert!(!fs.device_busy("ata0d0"));
        mount_fake(&mut fs, "ata0d0");
        assert!(fs.device_busy("ata0d0"));
        assert!(!fs.device_busy("ata0d1"));
    }

    #[test]
    fn read_zero_count_is_empty() {
        let mut fs = fresh_store();
        let handle = create(&mut fs, OWNER, "/a.txt");
        fs.write(OWNER, handle, 0, b"abc").unwrap();
        assert_eq!(fs.read(OWNER, handle, 0, 0).unwrap(), Vec::new());
    }

    #[test]
    fn mkdir_creates_a_real_directory_in_the_root_volume() {
        let mut fs = fresh_store();
        fs.mkdir("/disk").unwrap();
        assert_eq!(fs.stat("/disk").unwrap(), (EntityKind::Directory, 4096));
        assert_eq!(fs.list("/").unwrap(), b"disk".to_vec());
        assert_eq!(fs.mkdir("/disk"), Err(VfsError::AlreadyExists));
    }

    #[test]
    fn rmdir_removes_an_empty_directory() {
        let mut fs = fresh_store();
        fs.mkdir("/disk").unwrap();
        fs.rmdir("/disk").unwrap();
        assert_eq!(fs.stat("/disk"), Err(VfsError::NotFound));
    }

    #[test]
    fn mkdir_over_a_file_collides() {
        let mut fs = fresh_store();
        create(&mut fs, OWNER, "/a.txt");
        assert_eq!(fs.mkdir("/a.txt"), Err(VfsError::AlreadyExists));
    }

    #[test]
    fn rmdir_on_a_file_is_not_a_directory() {
        let mut fs = fresh_store();
        create(&mut fs, OWNER, "/a.txt");
        assert_eq!(fs.rmdir("/a.txt"), Err(VfsError::NotDirectory));
    }

    #[test]
    fn rmdir_refuses_a_directory_holding_files() {
        let mut fs = fresh_store();
        fs.mkdir("/disk").unwrap();
        let handle = fs.engine.create(b"/disk/x").unwrap();
        fs.engine.close(handle).unwrap();
        assert_eq!(fs.rmdir("/disk"), Err(VfsError::DirectoryNotEmpty));
    }
}
