use crate::blk::{FatSectors, RemoteBlock};
use crate::fstab::RootTarget;
use alloc::boxed::Box;
use alloc::collections::{BTreeMap, BTreeSet};
use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use block_protocol::client::{Block, ClientError};
use block_protocol::wire::BlockError;
use fs_core::engine::{DirItem, Engine, Kind, MetadataView};
use fs_core::error::FsError;
use rxfat::boot::{BootSector, FatType, SECTOR_BYTES};
use rxfat::device::BlockDevice as FatBlockDevice;
use rxfat::FileSystem as FatFileSystem;
use rxfs::device::BlockDevice;
use rxfs::device::MemoryDevice;
use rxfs::spec::DEFAULT_BYTES_PER_INODE;
use rxfs::spec::{BLOCK_BYTES, FS_MAGIC, SUPERBLOCK_BLOCK};
use rxfs::superblock::UUID_BYTES;
use rxfs::volume::{FileSystem, FormatParams, Handle as RxfsHandle, Stat as RxfsStat};
use system::rom::{RomError, RomImage};
use vfs_protocol::wire::{self, EntityKind, VfsError};

const VOLUME_BLOCKS: u64 = 2048;
const VOLUME_UUID: [u8; UUID_BYTES] = *b"rosx-ram-vol-001";
const VOLUME_NAME: &[u8] = b"rosx-ram";
const DEVICE_VOLUME_UUID: [u8; UUID_BYTES] = *b"rosx-ata-vol-001";
const DEVICE_VOLUME_NAME: &[u8] = b"rosx-hdd";

pub type ActiveVolume = VolumeEngine<MemoryDevice, FatSectors>;
pub type ActiveStore = VfsStore<ActiveVolume, RemoteBlock, FatSectors>;

pub struct AttachedDevice {
    pub id: String,
    pub service: String,
    pub device: u32,
    pub block_count: u64,
    pub block_size: u32,
    pub description: String,
}

pub enum VolumeEngine<D: BlockDevice, F: rxfat::device::BlockDevice> {
    Rxfs(Box<FileSystem<D>>),
    Fat(FatFileSystem<F>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VolumeHandle {
    Rxfs(RxfsHandle),
    Fat(rxfat::FatHandle),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VolumeMetadata {
    Rxfs(RxfsStat),
    Fat(rxfat::FatMetadata),
}

impl MetadataView for VolumeMetadata {
    fn kind(&self) -> Kind {
        match self {
            VolumeMetadata::Rxfs(stat) => stat.kind(),
            VolumeMetadata::Fat(meta) => meta.kind(),
        }
    }

    fn size(&self) -> u64 {
        match self {
            VolumeMetadata::Rxfs(stat) => stat.size(),
            VolumeMetadata::Fat(meta) => meta.size(),
        }
    }
}

impl<D: BlockDevice, F: rxfat::device::BlockDevice> Engine for VolumeEngine<D, F> {
    type Handle = VolumeHandle;
    type Metadata = VolumeMetadata;

    fn lookup(&mut self, path_bytes: &[u8]) -> Result<Self::Metadata, FsError> {
        match self {
            VolumeEngine::Rxfs(fs) => fs.lookup(path_bytes).map(VolumeMetadata::Rxfs),
            VolumeEngine::Fat(fs) => fs.lookup(path_bytes).map(VolumeMetadata::Fat),
        }
    }

    fn open(&mut self, path_bytes: &[u8]) -> Result<Self::Handle, FsError> {
        match self {
            VolumeEngine::Rxfs(fs) => fs.open(path_bytes).map(VolumeHandle::Rxfs),
            VolumeEngine::Fat(fs) => fs.open(path_bytes).map(VolumeHandle::Fat),
        }
    }

    fn create(&mut self, path_bytes: &[u8]) -> Result<Self::Handle, FsError> {
        match self {
            VolumeEngine::Rxfs(fs) => fs.create(path_bytes).map(VolumeHandle::Rxfs),
            VolumeEngine::Fat(fs) => fs.create(path_bytes).map(VolumeHandle::Fat),
        }
    }

    fn mkdir(&mut self, path_bytes: &[u8]) -> Result<(), FsError> {
        match self {
            VolumeEngine::Rxfs(fs) => fs.mkdir(path_bytes),
            VolumeEngine::Fat(fs) => fs.mkdir(path_bytes),
        }
    }

    fn rmdir(&mut self, path_bytes: &[u8]) -> Result<(), FsError> {
        match self {
            VolumeEngine::Rxfs(fs) => fs.rmdir(path_bytes),
            VolumeEngine::Fat(fs) => fs.rmdir(path_bytes),
        }
    }

    fn read(
        &mut self,
        handle: Self::Handle,
        offset: u64,
        buf: &mut [u8],
    ) -> Result<usize, FsError> {
        match (self, handle) {
            (VolumeEngine::Rxfs(fs), VolumeHandle::Rxfs(handle)) => {
                fs.read(handle, offset, buf)
            }
            (VolumeEngine::Fat(fs), VolumeHandle::Fat(handle)) => fs.read(handle, offset, buf),
            _ => Err(FsError::BadHandle),
        }
    }

    fn write(
        &mut self,
        handle: Self::Handle,
        offset: u64,
        data: &[u8],
    ) -> Result<usize, FsError> {
        match (self, handle) {
            (VolumeEngine::Rxfs(fs), VolumeHandle::Rxfs(handle)) => {
                fs.write(handle, offset, data)
            }
            (VolumeEngine::Fat(fs), VolumeHandle::Fat(handle)) => fs.write(handle, offset, data),
            _ => Err(FsError::BadHandle),
        }
    }

    fn stat(&mut self, handle: Self::Handle) -> Result<Self::Metadata, FsError> {
        match (self, handle) {
            (VolumeEngine::Rxfs(fs), VolumeHandle::Rxfs(handle)) => {
                fs.stat(handle).map(VolumeMetadata::Rxfs)
            }
            (VolumeEngine::Fat(fs), VolumeHandle::Fat(handle)) => {
                fs.stat(handle).map(VolumeMetadata::Fat)
            }
            _ => Err(FsError::BadHandle),
        }
    }

    fn close(&mut self, handle: Self::Handle) -> Result<(), FsError> {
        match (self, handle) {
            (VolumeEngine::Rxfs(fs), VolumeHandle::Rxfs(handle)) => fs.close(handle),
            (VolumeEngine::Fat(fs), VolumeHandle::Fat(handle)) => fs.close(handle),
            _ => Err(FsError::BadHandle),
        }
    }

    fn remove(&mut self, path_bytes: &[u8]) -> Result<(), FsError> {
        match self {
            VolumeEngine::Rxfs(fs) => fs.remove(path_bytes),
            VolumeEngine::Fat(fs) => fs.remove(path_bytes),
        }
    }

    fn readdir(&mut self, path_bytes: &[u8]) -> Result<Vec<DirItem>, FsError> {
        match self {
            VolumeEngine::Rxfs(fs) => fs.readdir(path_bytes),
            VolumeEngine::Fat(fs) => fs.readdir(path_bytes),
        }
    }

    fn sync(&mut self) -> Result<(), FsError> {
        match self {
            VolumeEngine::Rxfs(fs) => fs.sync(),
            VolumeEngine::Fat(fs) => fs.sync(),
        }
    }
}

struct MountedVolume<D: BlockDevice, F: rxfat::device::BlockDevice> {
    name: String,
    device: String,
    engine: VolumeEngine<D, F>,
}

enum OpenLocation<E: Engine> {
    Root(E::Handle),
    Mount { slot: usize, handle: VolumeHandle },
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

pub struct VfsStore<E: Engine, D: BlockDevice, F: rxfat::device::BlockDevice> {
    engine: E,
    devices: Vec<AttachedDevice>,
    mounts: Vec<MountedVolume<D, F>>,
    opens: BTreeMap<u32, OpenEntry<E>>,
    next_handle: u32,
    rom_names: BTreeSet<String>,
    rom_attached: bool,
    root_alias: Option<String>,
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
        FsError::NotADirectory => VfsError::NotDirectory,
        FsError::IsADirectory => VfsError::IsADirectory,
        FsError::DirectoryNotEmpty => VfsError::DirectoryNotEmpty,
        FsError::Unsupported => VfsError::Unsupported,
        _ => VfsError::ServerResource,
    }
}

fn map_block_error(error: ClientError) -> VfsError {
    match error {
        ClientError::Block(BlockError::NotFound) => VfsError::NotFound,
        ClientError::Block(BlockError::Unsupported) => VfsError::Unsupported,
        ClientError::NoConnection => VfsError::NotFound,
        _ => VfsError::ServerResource,
    }
}

fn map_mount_error(error: FsError) -> VfsError {
    match error {
        FsError::Corrupt | FsError::DeviceTooSmall => VfsError::NotFormatted,
        other => map_fs_error(other),
    }
}

#[derive(Debug, PartialEq, Eq)]
enum VolumeFormat {
    Rxfs,
    Fat,
    Unsupported,
    Foreign,
}

const RXFS_MAGIC_OFFSET: usize = SUPERBLOCK_BLOCK as usize * BLOCK_BYTES;
const ROOT_PROBE_SECTORS: usize =
    (SUPERBLOCK_BLOCK as usize * BLOCK_BYTES + FS_MAGIC.len()).div_ceil(SECTOR_BYTES as usize);

fn probe_format(head: &[u8], device_sectors: Option<u64>) -> VolumeFormat {
    if head.get(RXFS_MAGIC_OFFSET..RXFS_MAGIC_OFFSET + FS_MAGIC.len()) == Some(FS_MAGIC) {
        return VolumeFormat::Rxfs;
    }
    let Some(sectors) = device_sectors else {
        return VolumeFormat::Unsupported;
    };
    match BootSector::parse(head, sectors) {
        Ok(boot) if boot.fat_type == FatType::Fat12 => VolumeFormat::Unsupported,
        Ok(_) => VolumeFormat::Fat,
        Err(_) => VolumeFormat::Foreign,
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

impl VfsStore<ActiveVolume, RemoteBlock, FatSectors> {
    pub fn new() -> VfsStore<ActiveVolume, RemoteBlock, FatSectors> {
        let params = FormatParams {
            bytes_per_inode: DEFAULT_BYTES_PER_INODE,
            uuid: VOLUME_UUID,
            volume_name: VOLUME_NAME,
            created_at: 0,
        };
        let memory = FileSystem::format(MemoryDevice::new(VOLUME_BLOCKS), &params)
            .expect("fresh memory volume must format");
        VfsStore::from_engine(VolumeEngine::Rxfs(Box::new(memory)))
    }

    pub fn adopt_root(&mut self, target: &RootTarget) -> Result<(), String> {
        self.attach_device(&target.alias, &target.service, target.drive)
            .map_err(|error| format!("attach refused: {error:?}"))?;
        let mut view = match self.fat_view(&target.service, target.drive) {
            Ok(view) => view,
            Err(reason) => {
                self.detach(&target.alias);
                return Err(reason);
            }
        };
        let mut head = Vec::new();
        let probe_sectors = ROOT_PROBE_SECTORS.min(view.block_count() as usize);
        for sector in 0..probe_sectors as u64 {
            let mut block = [0u8; SECTOR_BYTES as usize];
            view.read_block(sector, &mut block)
                .map_err(|_| String::from("head unreadable"))?;
            head.extend_from_slice(&block);
        }
        match probe_format(&head, Some(view.block_count())) {
            VolumeFormat::Fat => {}
            VolumeFormat::Rxfs => return Err(String::from("rxfs device roots arrive with F5")),
            VolumeFormat::Unsupported => return Err(String::from("geometry cannot host FAT")),
            VolumeFormat::Foreign => return Err(String::from("no FAT volume")),
        }
        let engine = FatFileSystem::new(view).map_err(|error| format!("FAT rejected: {error:?}"))?;
        self.engine = VolumeEngine::Fat(engine);
        self.root_alias = Some(String::from(target.alias.as_str()));
        Ok(())
    }

    fn fat_view(&self, service: &str, drive: u32) -> Result<FatSectors, String> {
        let client = Block::connect(service)
            .map_err(|error| format!("dial {service} refused: {error:?}"))?;
        let geometry = match client.probe(drive) {
            Ok(geometry) => geometry,
            Err(error) => {
                client.disconnect();
                return Err(format!("drive {drive} probe refused: {error:?}"));
            }
        };
        let head_block = match client.read_blocks(drive, 0, 1) {
            Ok(block) => block,
            Err(error) => {
                client.disconnect();
                return Err(format!("drive {drive} head unreadable: {error:?}"));
            }
        };
        FatSectors::new_view(client, drive, &geometry, crate::blk::first_partition(&head_block))
            .ok_or_else(|| format!("drive {drive} geometry cannot host a FAT view"))
    }

    fn detach(&mut self, id: &str) {
        self.devices.retain(|attached| attached.id != id);
    }

    pub fn attach_device(
        &mut self,
        id: &str,
        service: &str,
        device: u32,
    ) -> Result<(), VfsError> {
        if !wire::valid_name(id) {
            return Err(VfsError::InvalidPath);
        }
        if self.devices.iter().any(|attached| attached.id == id) {
            return Err(VfsError::Busy);
        }
        if self
            .devices
            .iter()
            .any(|attached| attached.service == service && attached.device == device)
        {
            return Err(VfsError::Busy);
        }
        let client = Block::connect(service).map_err(map_block_error)?;
        let geometry = client.probe(device).map_err(map_block_error)?;
        client.disconnect();
        if crate::blk::device_blocks_per_block(geometry.block_size).is_none() {
            return Err(VfsError::Unsupported);
        }
        self.devices.push(AttachedDevice {
            id: String::from(id),
            service: String::from(service),
            device,
            block_count: geometry.block_count,
            block_size: geometry.block_size,
            description: geometry.description,
        });
        Ok(())
    }

    fn attached(&self, id: &str) -> Result<&AttachedDevice, VfsError> {
        self.devices
            .iter()
            .find(|attached| attached.id == id)
            .ok_or(VfsError::NotFound)
    }

    pub fn format_device(&mut self, id: &str) -> Result<(), VfsError> {
        let attached = self.attached(id)?;
        if self.device_busy(id) {
            return Err(VfsError::Busy);
        }
        let (service, device) = (attached.service.clone(), attached.device);
        let client = Block::connect(&service).map_err(map_block_error)?;
        let geometry = client.probe(device).map_err(map_block_error)?;
        let disk = RemoteBlock::new(client, device, &geometry).ok_or(VfsError::Unsupported)?;
        let params = FormatParams {
            bytes_per_inode: DEFAULT_BYTES_PER_INODE,
            uuid: DEVICE_VOLUME_UUID,
            volume_name: DEVICE_VOLUME_NAME,
            created_at: 0,
        };
        let fs = FileSystem::format(disk, &params).map_err(map_fs_error)?;
        fs.unmount()
            .map(|disk| disk.into_client().disconnect())
            .map_err(map_fs_error)
    }

    pub fn mount_device(&mut self, id: &str, dir: &str) -> Result<(), VfsError> {
        if !wire::valid_name(dir) {
            return Err(VfsError::InvalidPath);
        }
        if self.device_busy(id) || self.mounts.iter().any(|mount| mount.name == dir) {
            return Err(VfsError::Busy);
        }
        let mount_point = self
            .engine
            .lookup(&volume_path(dir))
            .map_err(map_fs_error)?;
        if mount_point.kind() != Kind::Directory {
            return Err(VfsError::NotDirectory);
        }
        let attached = self.attached(id)?;
        let (service, device) = (attached.service.clone(), attached.device);
        let client = Block::connect(&service).map_err(map_block_error)?;
        let geometry = client.probe(device).map_err(map_block_error)?;
        let head_blocks = crate::blk::probe_head_blocks(geometry.block_size)
            .min(geometry.block_count as usize)
            .max(1);
        let head = client
            .read_blocks(device, 0, head_blocks)
            .map_err(map_block_error)?;
        let device_sectors =
            crate::blk::fat_sector_count(geometry.block_count, geometry.block_size);
        let engine = match probe_format(&head, device_sectors) {
            VolumeFormat::Rxfs => {
                let disk = RemoteBlock::new(client, device, &geometry).ok_or(VfsError::Unsupported)?;
                let fs = FileSystem::mount(disk).map_err(map_mount_error)?;
                VolumeEngine::Rxfs(Box::new(fs))
            }
            VolumeFormat::Fat => {
                let sectors = FatSectors::new(client, device, &geometry).ok_or(VfsError::Unsupported)?;
                let fs = FatFileSystem::new(sectors).map_err(map_mount_error)?;
                VolumeEngine::Fat(fs)
            }
            VolumeFormat::Unsupported => {
                client.disconnect();
                return Err(VfsError::Unsupported);
            }
            VolumeFormat::Foreign => {
                client.disconnect();
                return Err(VfsError::NotFormatted);
            }
        };
        self.mounts.push(MountedVolume {
            name: String::from(dir),
            device: String::from(id),
            engine,
        });
        Ok(())
    }
}

impl<E: Engine, D: BlockDevice, F: rxfat::device::BlockDevice> VfsStore<E, D, F> {
    fn from_engine(engine: E) -> VfsStore<E, D, F> {
        VfsStore {
            engine,
            devices: Vec::new(),
            mounts: Vec::new(),
            opens: BTreeMap::new(),
            next_handle: 1,
            rom_names: BTreeSet::new(),
            rom_attached: false,
            root_alias: None,
        }
    }

    fn route(&self, path: &str) -> Result<Target, VfsError> {
        let (head, rest) = match path.split_once('/') {
            Some(split) => split,
            None => (path, ""),
        };
        match self.mounts.iter().position(|mount| mount.name == head) {
            Some(slot) => Ok(Target::Volume(slot, format!("/{rest}"))),
            None if rest.is_empty() => Ok(Target::Root),
            None => Err(VfsError::NotFound),
        }
    }

    fn volume_mut(&mut self, slot: usize) -> Result<&mut VolumeEngine<D, F>, VfsError> {
        self.mounts
            .get_mut(slot)
            .map(|mounted| &mut mounted.engine)
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

    pub fn devices(&self) -> &[AttachedDevice] {
        &self.devices
    }

    pub fn mounted_at(&self, id: &str) -> Option<&str> {
        self.mounts
            .iter()
            .find(|mount| mount.device == id)
            .map(|mount| mount.name.as_str())
    }

    pub fn device_busy(&self, id: &str) -> bool {
        self.root_alias.as_deref() == Some(id) || self.mounts.iter().any(|mount| mount.device == id)
    }

    pub fn is_root_device(&self, id: &str) -> bool {
        self.root_alias.as_deref() == Some(id)
    }

    pub fn attach_rom(&mut self, image: &[u8]) -> Result<(), RomMountError> {
        if self.root_alias.is_some() {
            return Ok(());
        }
        if self.rom_attached {
            return Err(RomMountError::AlreadyAttached);
        }
        let parsed = RomImage::parse(image).map_err(RomMountError::Format)?;
        let mut staged: Vec<(&str, usize, usize)> = Vec::new();
        for entry in parsed.files() {
            if !wire::valid_name(entry.name) {
                return Err(RomMountError::InvalidName(String::from(entry.name)));
            }
            if self.engine.lookup(&volume_path(entry.name)).is_ok() {
                return Err(RomMountError::Duplicate(String::from(entry.name)));
            }
            staged.push((entry.name, entry.offset, entry.len));
        }
        for (name, offset, len) in staged {
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
            self.rom_names.insert(String::from(name));
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
                if mode & wire::MODE_CREATE != 0 && self.root_alias.is_some() {
                    return Err(VfsError::Unsupported);
                }
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
        let location = entry.location;
        if matches!(location, OpenLocation::Root(_)) && self.root_alias.is_some() {
            return Err(VfsError::Unsupported);
        }
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
                if self.root_alias.is_some() {
                    return Err(VfsError::Unsupported);
                }
                if self.rom_names.contains(name) {
                    return Err(VfsError::NotPermitted);
                }
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
                if self.root_alias.is_some() {
                    return match self.engine.lookup(&volume_path(path)) {
                        Ok(_) => Err(VfsError::AlreadyExists),
                        Err(FsError::NotFound) => Err(VfsError::Unsupported),
                        Err(error) => Err(map_fs_error(error)),
                    };
                }
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
                if self.root_alias.is_some() {
                    return Err(VfsError::Unsupported);
                }
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
    use fs_core::error::IoError;

    const OWNER: u32 = 0x0010_0001;
    const FAT_SECTOR_BYTES: usize = 512;

    type TestStore = VfsStore<ActiveVolume, MemoryDevice, TestSectors>;

    struct TestSectors {
        bytes: Vec<u8>,
    }

    impl rxfat::device::BlockDevice for TestSectors {
        fn block_count(&self) -> u64 {
            self.bytes.len() as u64 / FAT_SECTOR_BYTES as u64
        }

        fn read_block(&mut self, block: u64, buf: &mut [u8]) -> Result<(), IoError> {
            let at = block as usize * FAT_SECTOR_BYTES;
            let Some(region) = self.bytes.get(at..at + FAT_SECTOR_BYTES) else {
                return Err(IoError::Device);
            };
            if buf.len() != FAT_SECTOR_BYTES {
                return Err(IoError::Device);
            }
            buf.copy_from_slice(region);
            Ok(())
        }

        fn write_block(&mut self, block: u64, buf: &[u8]) -> Result<(), IoError> {
            let at = block as usize * FAT_SECTOR_BYTES;
            if buf.len() != FAT_SECTOR_BYTES {
                return Err(IoError::Device);
            }
            let Some(region) = self.bytes.get_mut(at..at + FAT_SECTOR_BYTES) else {
                return Err(IoError::Device);
            };
            region.copy_from_slice(buf);
            Ok(())
        }

        fn flush(&mut self) -> Result<(), IoError> {
            Ok(())
        }
    }

    const FAT16_TOTAL_SECTORS: u64 = 49 + 4_086;
    const FAT16_ROOT_SECTOR: usize = 17;
    const FAT16_DATA_SECTOR: usize = 49;

    fn fat16_image() -> TestSectors {
        let mut bytes = vec![0u8; FAT16_TOTAL_SECTORS as usize * FAT_SECTOR_BYTES];
        let put16 = |bytes: &mut [u8], offset: usize, value: u16| {
            bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes())
        };
        put16(&mut bytes, 0x0B, 512);
        bytes[0x0D] = 1;
        put16(&mut bytes, 0x0E, 1);
        bytes[0x10] = 2;
        put16(&mut bytes, 0x11, 512);
        put16(&mut bytes, 0x13, FAT16_TOTAL_SECTORS as u16);
        put16(&mut bytes, 0x16, 8);
        bytes[0x36..0x3E].copy_from_slice(b"FAT16   ");
        bytes[0x1FE] = 0x55;
        bytes[0x1FF] = 0xAA;
        put16(&mut bytes, FAT_SECTOR_BYTES + 2 * 2, 0xFFF8);
        let mut slot = [b' '; 32];
        slot[..8].copy_from_slice(b"HELLO   ");
        slot[8..11].copy_from_slice(b"TXT");
        slot[0x14..0x16].copy_from_slice(&0u16.to_le_bytes());
        slot[0x1A..0x1C].copy_from_slice(&2u16.to_le_bytes());
        slot[0x1C..0x20].copy_from_slice(&5u32.to_le_bytes());
        bytes[FAT16_ROOT_SECTOR * FAT_SECTOR_BYTES..FAT16_ROOT_SECTOR * FAT_SECTOR_BYTES + 32]
            .copy_from_slice(&slot);
        bytes[FAT16_DATA_SECTOR * FAT_SECTOR_BYTES..FAT16_DATA_SECTOR * FAT_SECTOR_BYTES + 5]
            .copy_from_slice(b"hello");
        TestSectors { bytes }
    }

    fn fresh_store() -> TestStore {
        let params = FormatParams {
            bytes_per_inode: DEFAULT_BYTES_PER_INODE,
            uuid: VOLUME_UUID,
            volume_name: VOLUME_NAME,
            created_at: 0,
        };
        let memory = FileSystem::format(MemoryDevice::new(VOLUME_BLOCKS), &params)
            .expect("fresh memory volume must format");
        VfsStore::from_engine(VolumeEngine::Rxfs(Box::new(memory)))
    }

    fn adopt_alias(fs: &mut TestStore) {
        fs.devices.push(AttachedDevice {
            id: String::from("root0"),
            service: String::from("ATA"),
            device: 0,
            block_count: 2_048,
            block_size: 512,
            description: String::from("fake root drive"),
        });
        fs.root_alias = Some(String::from("root0"));
    }

    fn mount_fake(fs: &mut TestStore, name: &str) {
        fs.mkdir(name).unwrap();
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
            engine: VolumeEngine::Rxfs(Box::new(volume)),
        });
    }

    fn mount_fat_fake(fs: &mut TestStore, name: &str) {
        fs.mkdir(name).unwrap();
        let volume = FatFileSystem::new(fat16_image()).expect("synthetic fat16 volume must open");
        fs.mounts.push(MountedVolume {
            name: String::from(name),
            device: String::from(name),
            engine: VolumeEngine::Fat(volume),
        });
    }

    const INTRUDER: u32 = 0x0020_0002;

    fn create(fs: &mut TestStore, owner: u32, name: &str) -> u32 {
        fs.open(owner, wire::MODE_CREATE | wire::MODE_WRITE, name)
            .expect("create must succeed")
    }

    fn contents(fs: &mut TestStore, owner: u32, handle: u32) -> Vec<u8> {
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
    fn write_to_rom_file_is_not_permitted_even_when_empty() {
        let mut fs = fresh_store();
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
        let mut fs = fresh_store();
        fs.attach_rom(&fabricated_rom()).unwrap();
        assert_eq!(fs.delete("alpha.bin"), Err(VfsError::NotPermitted));
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
    fn attach_rom_rejects_name_invalid_for_vfs() {
        let mut fs = fresh_store();
        let image = system::rom::encode(&[("bad\nname", b"x".as_slice())]);
        assert!(matches!(
            fs.attach_rom(&image),
            Err(RomMountError::InvalidName(ref name)) if name == "bad\nname"
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
    fn unknown_mount_prefix_is_not_found_for_every_operation() {
        let mut fs = fresh_store();
        assert_eq!(fs.open(OWNER, 0, "floppy/x"), Err(VfsError::NotFound));
        assert_eq!(fs.delete("floppy/x"), Err(VfsError::NotFound));
        assert_eq!(fs.stat("floppy/x"), Err(VfsError::NotFound));
        assert_eq!(fs.list_dir("floppy/x"), Err(VfsError::NotFound));
        assert_eq!(fs.mkdir("floppy/x"), Err(VfsError::NotFound));
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
    fn mount_device_of_unknown_dir_is_not_found_before_touching_the_service() {
        let mut fs = ActiveStore::new();
        assert_eq!(fs.mount_device("ata0", "floppy"), Err(VfsError::NotFound));
    }

    fn fake_attached(fs: &mut TestStore, id: &str, service: &str, device: u32) {
        fs.devices.push(AttachedDevice {
            id: String::from(id),
            service: String::from(service),
            device,
            block_count: 102400,
            block_size: 512,
            description: String::from("QEMU HARDDISK"),
        });
    }

    #[test]
    fn attached_devices_and_mount_points_are_visible() {
        let mut fs = fresh_store();
        assert!(fs.devices().is_empty());
        assert_eq!(fs.mounted_at("ata0"), None);
        fake_attached(&mut fs, "ata0", "ATA", 1);
        assert_eq!(fs.devices().len(), 1);
        assert_eq!(fs.devices()[0].service, "ATA");
        mount_fake(&mut fs, "ata0");
        assert_eq!(fs.mounted_at("ata0"), Some("ata0"));
        assert_eq!(fs.mounted_at("sata1"), None);
    }

    #[test]
    fn mount_and_format_of_unknown_device_id_are_not_found_before_touching_the_service() {
        let mut fs = ActiveStore::new();
        fs.mkdir("hdd").unwrap();
        assert_eq!(fs.mount_device("ghost", "hdd"), Err(VfsError::NotFound));
        assert_eq!(fs.format_device("ghost"), Err(VfsError::NotFound));
    }

    #[test]
    fn a_live_volume_marks_its_device_busy() {
        let mut fs = fresh_store();
        fake_attached(&mut fs, "ata0", "ATA", 0);
        assert!(!fs.device_busy("ata0"));
        mount_fake(&mut fs, "ata0");
        assert!(fs.device_busy("ata0"));
        assert!(!fs.device_busy("sata1"));
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

    #[test]
    fn probe_recognises_rxfs_magic_at_the_superblock_block() {
        let mut head = vec![0u8; RXFS_MAGIC_OFFSET + FS_MAGIC.len()];
        head[RXFS_MAGIC_OFFSET..RXFS_MAGIC_OFFSET + FS_MAGIC.len()].copy_from_slice(FS_MAGIC);
        assert_eq!(probe_format(&head, None), VolumeFormat::Rxfs);
    }

    #[test]
    fn probe_ignores_rxfs_magic_misplaced_at_byte_zero() {
        let mut head = vec![0u8; RXFS_MAGIC_OFFSET + FS_MAGIC.len()];
        head[..FS_MAGIC.len()].copy_from_slice(FS_MAGIC);
        assert_eq!(probe_format(&head, Some(65_536)), VolumeFormat::Foreign);
    }

    #[test]
    fn probe_recognises_fat16_boot_sectors() {
        let image = fat16_image();
        let head = &image.bytes[..FAT_SECTOR_BYTES];
        assert_eq!(probe_format(head, Some(FAT16_TOTAL_SECTORS)), VolumeFormat::Fat);
    }

    #[test]
    fn probe_answers_unsupported_for_fat12_and_sectorless_devices() {
        let image = fat16_image();
        let mut head = image.bytes[..FAT_SECTOR_BYTES].to_vec();
        let put16 = |head: &mut [u8], offset: usize, value: u16| {
            head[offset..offset + 2].copy_from_slice(&value.to_le_bytes())
        };
        put16(&mut head, 0x11, 224);
        put16(&mut head, 0x13, 0);
        put16(&mut head, 0x16, 16);
        head[0x20..0x24].copy_from_slice(&(47u32 + 4_084).to_le_bytes());
        assert_eq!(probe_format(&head, Some(100_000)), VolumeFormat::Unsupported);
        let clean = &image.bytes[..FAT_SECTOR_BYTES];
        assert_eq!(probe_format(clean, None), VolumeFormat::Unsupported);
    }

    #[test]
    fn probe_answers_foreign_for_zeroed_and_signatureless_media() {
        let zeros = vec![0u8; FAT_SECTOR_BYTES];
        assert_eq!(probe_format(&zeros, Some(65_536)), VolumeFormat::Foreign);
        let image = fat16_image();
        let mut head = image.bytes[..FAT_SECTOR_BYTES].to_vec();
        head[0x1FE] = 0x00;
        assert_eq!(probe_format(&head, Some(FAT16_TOTAL_SECTORS)), VolumeFormat::Foreign);
    }

    #[test]
    fn fat_mount_lists_and_reads_the_volume_through_routing() {
        let mut fs = fresh_store();
        mount_fat_fake(&mut fs, "fat");
        assert_eq!(fs.list_dir("fat").unwrap(), b"hello.txt");
        assert_eq!(fs.stat("fat/hello.txt").unwrap(), (EntityKind::File, 5));
        let handle = fs.open(OWNER, 0, "fat/hello.txt").expect("read open must succeed");
        assert_eq!(contents(&mut fs, OWNER, handle), b"hello".to_vec());
        fs.close(OWNER, handle).unwrap();
    }

    #[test]
    fn fat_mount_reads_are_owner_scoped() {
        let mut fs = fresh_store();
        mount_fat_fake(&mut fs, "fat");
        let handle = fs.open(OWNER, 0, "fat/hello.txt").unwrap();
        assert_eq!(
            fs.read(INTRUDER, handle, 0, 16),
            Err(VfsError::BadHandle)
        );
    }

    #[test]
    fn fat_mount_mutations_flow_through_daemon_routing() {
        let mut fs = fresh_store();
        mount_fat_fake(&mut fs, "fat");
        let handle = fs
            .open(OWNER, wire::MODE_CREATE | wire::MODE_WRITE, "fat/new.txt")
            .expect("create on a FAT volume");
        fs.write(OWNER, handle, 0, b"written").expect("write flows through");
        fs.close(OWNER, handle).unwrap();
        assert_eq!(fs.stat("fat/new.txt").unwrap(), (EntityKind::File, 7));
        fs.mkdir("fat/sub").expect("mkdir on a FAT volume");
        let existing = fs.open(OWNER, wire::MODE_WRITE, "fat/hello.txt").unwrap();
        assert_eq!(fs.write(OWNER, existing, 0, b"j"), Ok(1));
        fs.close(OWNER, existing).unwrap();
        assert_eq!(fs.delete("fat/hello.txt"), Ok(()));
        assert_eq!(fs.stat("fat/hello.txt"), Err(VfsError::NotFound));
        assert_eq!(fs.list_dir("fat").unwrap(), b"new.txt\nsub");
        let reopened = fs.open(OWNER, 0, "fat/new.txt").expect("reopen");
        assert_eq!(contents(&mut fs, OWNER, reopened), b"written".to_vec());
        fs.close(OWNER, reopened).unwrap();
    }

    #[test]
    fn fat_mount_points_keep_the_same_guards_as_rxfs_ones() {
        let mut fs = fresh_store();
        mount_fat_fake(&mut fs, "fat");
        assert_eq!(fs.delete("fat"), Err(VfsError::NotPermitted));
        assert_eq!(fs.rmdir("fat"), Err(VfsError::NotPermitted));
        assert_eq!(fs.mounted_at("fat"), Some("fat"));
        assert!(fs.device_busy("fat"));
    }

    #[test]
    fn fat_mount_is_invisible_on_the_root_and_unknown_files_stay_not_found() {
        let mut fs = fresh_store();
        mount_fat_fake(&mut fs, "fat");
        assert_eq!(fs.list().unwrap(), b"fat");
        assert_eq!(fs.open(OWNER, 0, "fat/ghost.txt"), Err(VfsError::NotFound));
        assert_eq!(fs.stat("fat/ghost.txt"), Err(VfsError::NotFound));
    }

    #[test]
    fn adopted_root_still_reads_but_refuses_create_open() {
        let mut fs = fresh_store();
        let handle = create(&mut fs, OWNER, "note");
        fs.close(OWNER, handle).unwrap();
        adopt_alias(&mut fs);
        let read = fs
            .open(OWNER, 0, "note")
            .expect("a device root is still readable");
        fs.close(OWNER, read).unwrap();
        assert_eq!(fs.open(OWNER, wire::MODE_CREATE, "fresh"), Err(VfsError::Unsupported));
    }

    #[test]
    fn adopted_root_refuses_writes() {
        let mut fs = fresh_store();
        let handle = create(&mut fs, OWNER, "note");
        fs.write(OWNER, handle, 0, b"abc").unwrap();
        adopt_alias(&mut fs);
        assert_eq!(fs.write(OWNER, handle, 0, b"abc"), Err(VfsError::Unsupported));
    }

    #[test]
    fn adopted_root_mkdir_answers_existing_dirs_honestly() {
        let mut fs = fresh_store();
        fs.mkdir("data").unwrap();
        adopt_alias(&mut fs);
        assert_eq!(fs.mkdir("data"), Err(VfsError::AlreadyExists));
        assert_eq!(fs.mkdir("more"), Err(VfsError::Unsupported));
    }

    #[test]
    fn adopted_root_refuses_rmdir_and_delete() {
        let mut fs = fresh_store();
        fs.mkdir("data").unwrap();
        let handle = create(&mut fs, OWNER, "note");
        fs.close(OWNER, handle).unwrap();
        adopt_alias(&mut fs);
        assert_eq!(fs.rmdir("data"), Err(VfsError::Unsupported));
        assert_eq!(fs.delete("note"), Err(VfsError::Unsupported));
    }

    #[test]
    fn adopted_root_skips_rom_ingestion() {
        let pairs: Vec<(&str, &[u8])> = vec![("xfile", b"payload".as_slice())];
        let image = system::rom::encode(&pairs);
        let mut fs = fresh_store();
        adopt_alias(&mut fs);
        fs.attach_rom(&image).expect("ingestion is a no-op on a device root");
        assert!(!fs.rom_attached);
        assert_eq!(fs.list().unwrap(), b"");
    }

    #[test]
    fn adopted_root_device_reports_busy_and_root_state() {
        let mut fs = fresh_store();
        adopt_alias(&mut fs);
        assert!(fs.device_busy("root0"));
        assert!(fs.is_root_device("root0"));
        assert!(!fs.is_root_device("other"));
    }
}
