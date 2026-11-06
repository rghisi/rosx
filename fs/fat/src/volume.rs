use crate::boot::{BootSector, FatType};
use crate::device::BlockDevice;
use crate::dir::{self, DirEntry, DirLocation};
use crate::fat;
use crate::file;
use crate::path::{self, PathParts};
use alloc::vec::Vec;
use fs_core::engine::{DirItem, Engine, Kind, MetadataView};
use fs_core::error::FsError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FatHandle {
    slot: u32,
    generation: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FatMetadata {
    kind: Kind,
    size: u64,
}

impl MetadataView for FatMetadata {
    fn kind(&self) -> Kind {
        self.kind
    }

    fn size(&self) -> u64 {
        self.size
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct OpenEntry {
    generation: u32,
    first_cluster: u32,
    size: u64,
    chain: Vec<u32>,
    parent: DirLocation,
    slot_sector: u64,
    slot_offset: usize,
    slot_name: [u8; 11],
    attrs: u8,
}

const ATTR_ARCHIVE: u8 = 0x20;
const ATTR_DIRECTORY: u8 = 0x10;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Resolved {
    is_directory: bool,
    first_cluster: u32,
    size: u64,
}

#[derive(Debug)]
pub struct FileSystem<D: BlockDevice> {
    boot: BootSector,
    device: D,
    opens: Vec<Option<OpenEntry>>,
    next_generation: u32,
    free_clusters: Option<u32>,
}

impl<D: BlockDevice> FileSystem<D> {
    pub fn new(mut device: D) -> Result<FileSystem<D>, FsError> {
        let device_sectors = device.block_count();
        let mut boot_block = [0u8; 512];
        device.read_block(0, &mut boot_block).map_err(FsError::Io)?;
        let boot = BootSector::parse(&boot_block, device_sectors)?;
        if boot.fat_type == FatType::Fat12 {
            return Err(FsError::Unsupported);
        }
        Ok(FileSystem {
            boot,
            device,
            opens: Vec::new(),
            next_generation: 1,
            free_clusters: None,
        })
    }

    pub fn lookup(&mut self, path_bytes: &[u8]) -> Result<FatMetadata, FsError> {
        let parts = path::parse(path_bytes)?;
        let resolved = self.resolve(&parts)?;
        Ok(FatMetadata {
            kind: if resolved.is_directory {
                Kind::Directory
            } else {
                Kind::Regular
            },
            size: resolved.size,
        })
    }

    pub fn open(&mut self, path_bytes: &[u8]) -> Result<FatHandle, FsError> {
        let parts = path::parse(path_bytes)?;
        if parts.is_root() {
            return Err(FsError::IsADirectory);
        }
        let parent = self.parent_location(&parts)?;
        let name = parts.last_segment().ok_or(FsError::InvalidPath)?;
        let located = dir::locate_slot(&mut self.device, &self.boot, parent, name)?;
        if located.entry.is_directory {
            return Err(FsError::IsADirectory);
        }
        let chain = self.data_chain(located.entry.first_cluster)?;
        let generation = self.next_generation;
        self.next_generation = self.next_generation.wrapping_add(1);
        let slot = self.reserve_open_slot();
        self.opens[slot] = Some(OpenEntry {
            generation,
            first_cluster: located.entry.first_cluster,
            size: located.entry.size,
            chain,
            parent,
            slot_sector: located.sector,
            slot_offset: located.offset,
            slot_name: located.slot_name,
            attrs: located.attrs,
        });
        Ok(FatHandle {
            slot: slot as u32,
            generation,
        })
    }

    pub fn read(
        &mut self,
        handle: FatHandle,
        offset: u64,
        buf: &mut [u8],
    ) -> Result<usize, FsError> {
        let entry = self.live_entry(handle)?;
        let first_cluster = entry.first_cluster;
        let size = entry.size;
        file::read_file(&mut self.device, &self.boot, first_cluster, size, offset, buf)
    }

    pub fn stat(&mut self, handle: FatHandle) -> Result<FatMetadata, FsError> {
        let entry = self.live_entry(handle)?;
        Ok(FatMetadata {
            kind: Kind::Regular,
            size: entry.size,
        })
    }

    pub fn close(&mut self, handle: FatHandle) -> Result<(), FsError> {
        self.live_entry(handle)?;
        self.opens[handle.slot as usize] = None;
        Ok(())
    }

    pub fn readdir(&mut self, path_bytes: &[u8]) -> Result<Vec<DirItem>, FsError> {
        let parts = path::parse(path_bytes)?;
        let location = self.resolve_directory(&parts)?;
        let entries = dir::read_directory(&mut self.device, &self.boot, location)?;
        Ok(entries
            .into_iter()
            .map(|entry| DirItem {
                kind: if entry.is_directory {
                    Kind::Directory
                } else {
                    Kind::Regular
                },
                name: entry.name,
            })
            .collect())
    }

    fn live_entry(&self, handle: FatHandle) -> Result<&OpenEntry, FsError> {
        self.opens
            .get(handle.slot as usize)
            .and_then(|slot| slot.as_ref())
            .filter(|entry| entry.generation == handle.generation)
            .ok_or(FsError::BadHandle)
    }

    fn resolve(&mut self, parts: &PathParts) -> Result<Resolved, FsError> {
        if parts.is_root() {
            return Ok(Resolved {
                is_directory: true,
                first_cluster: 0,
                size: 0,
            });
        }
        let mut location = DirLocation::from_root(self.boot.root);
        for index in 0..parts.len() {
            let name = parts.get(index).ok_or(FsError::InvalidPath)?;
            let entry = self.find_entry(location, name)?;
            if index + 1 == parts.len() {
                return Ok(Resolved {
                    is_directory: entry.is_directory,
                    first_cluster: entry.first_cluster,
                    size: entry.size,
                });
            }
            if !entry.is_directory {
                return Err(FsError::NotADirectory);
            }
            location = DirLocation::Chain(entry.first_cluster);
        }
        Err(FsError::InvalidPath)
    }

    fn resolve_directory(&mut self, parts: &PathParts) -> Result<DirLocation, FsError> {
        if parts.is_root() {
            return Ok(DirLocation::from_root(self.boot.root));
        }
        let resolved = self.resolve(parts)?;
        if !resolved.is_directory {
            return Err(FsError::NotADirectory);
        }
        Ok(DirLocation::Chain(resolved.first_cluster))
    }

    fn find_entry(&mut self, location: DirLocation, name: &[u8]) -> Result<DirEntry, FsError> {
        let entries = dir::read_directory(&mut self.device, &self.boot, location)?;
        entries
            .into_iter()
            .find(|entry| dir::name_matches(&entry.name, name))
            .ok_or(FsError::NotFound)
    }

    fn parent_location(&mut self, parts: &PathParts) -> Result<DirLocation, FsError> {
        let mut location = DirLocation::from_root(self.boot.root);
        for index in 0..parts.len() - 1 {
            let name = parts.get(index).ok_or(FsError::InvalidPath)?;
            let entry = self.find_entry(location, name)?;
            if !entry.is_directory {
                return Err(FsError::NotADirectory);
            }
            location = DirLocation::Chain(entry.first_cluster);
        }
        Ok(location)
    }

    fn reserve_open_slot(&mut self) -> usize {
        match self.opens.iter().position(|slot| slot.is_none()) {
            Some(slot) => slot,
            None => {
                self.opens.push(None);
                self.opens.len() - 1
            }
        }
    }

    fn data_chain(&mut self, first_cluster: u32) -> Result<Vec<u32>, FsError> {
        if first_cluster < 2 {
            return Ok(Vec::new());
        }
        fat::cluster_chain(&mut self.device, &self.boot, first_cluster)
    }

    fn cluster_bytes(&self) -> u64 {
        u64::from(self.boot.sectors_per_cluster) * u64::from(self.boot.bytes_per_sector)
    }

    fn allocate_slot(&mut self, location: DirLocation) -> Result<(u64, usize), FsError> {
        loop {
            let sectors = dir::entry_sectors(&mut self.device, &self.boot, location)?;
            let limit = match location {
                DirLocation::Sectors { entry_count, .. } => Some(entry_count),
                DirLocation::Chain(_) => None,
            };
            if let Some(spot) = dir::find_slot_spot(&mut self.device, &sectors, limit)? {
                return Ok(match spot {
                    dir::SlotSpot::Reuse(sector, offset) => (sector, offset),
                    dir::SlotSpot::Append(sector, offset) => (sector, offset),
                });
            }
            match location {
                DirLocation::Sectors { .. } => return Err(FsError::NoSpace),
                DirLocation::Chain(head) => {
                    let chain = fat::cluster_chain(&mut self.device, &self.boot, head)?;
                    let tail = *chain.last().ok_or(FsError::Corrupt)?;
                    self.reserve_clusters(1)?;
                    fat::append_chain(&mut self.device, &self.boot, tail, 1)?;
                }
            }
        }
    }

    fn tracks_free_clusters(&self) -> bool {
        self.boot.fat_type == FatType::Fat32 && self.boot.fsinfo_sector != 0
    }

    fn reserve_clusters(&mut self, count: u32) -> Result<(), FsError> {
        if !self.tracks_free_clusters() {
            return Ok(());
        }
        let base = match self.free_clusters {
            Some(free) => free,
            None => fat::count_free_clusters(&mut self.device, &self.boot)?,
        };
        let remaining = base.checked_sub(count).ok_or(FsError::NoSpace)?;
        self.free_clusters = Some(remaining);
        fat::write_free_cluster_count(&mut self.device, &self.boot, remaining)
    }

    fn release_clusters(&mut self, count: u32) -> Result<(), FsError> {
        if count == 0 || !self.tracks_free_clusters() {
            return Ok(());
        }
        let base = match self.free_clusters {
            Some(free) => free,
            None => fat::count_free_clusters(&mut self.device, &self.boot)?,
        };
        let remaining = base.saturating_add(count);
        self.free_clusters = Some(remaining);
        fat::write_free_cluster_count(&mut self.device, &self.boot, remaining)
    }

    pub fn create(&mut self, path_bytes: &[u8]) -> Result<FatHandle, FsError> {
        let parts = path::parse(path_bytes)?;
        if parts.is_root() {
            return Err(FsError::IsADirectory);
        }
        let name = parts.last_segment().ok_or(FsError::InvalidPath)?;
        let folded = dir::fold_to_slot(name)?;
        let parent = self.parent_location(&parts)?;
        if dir::locate_slot(&mut self.device, &self.boot, parent, name).is_ok() {
            return Err(FsError::AlreadyExists);
        }
        let (sector, offset) = self.allocate_slot(parent)?;
        let slot = dir::make_slot_bytes(folded, ATTR_ARCHIVE, 0, 0);
        dir::write_slot_bytes(&mut self.device, sector, offset, &slot)?;
        let mut slot_name = [0u8; 11];
        slot_name.copy_from_slice(&slot[..11]);
        let generation = self.next_generation;
        self.next_generation = self.next_generation.wrapping_add(1);
        let slot_index = self.reserve_open_slot();
        self.opens[slot_index] = Some(OpenEntry {
            generation,
            first_cluster: 0,
            size: 0,
            chain: Vec::new(),
            parent,
            slot_sector: sector,
            slot_offset: offset,
            slot_name,
            attrs: ATTR_ARCHIVE,
        });
        Ok(FatHandle {
            slot: slot_index as u32,
            generation,
        })
    }

    pub fn write(
        &mut self,
        handle: FatHandle,
        offset: u64,
        data: &[u8],
    ) -> Result<usize, FsError> {
        self.live_entry(handle)?;
        if data.is_empty() {
            return Ok(0);
        }
        let end = offset
            .checked_add(data.len() as u64)
            .ok_or(FsError::FileTooLarge)?;
        if end > u64::from(u32::MAX) {
            return Err(FsError::FileTooLarge);
        }
        let snapshot = self.opens[handle.slot as usize]
            .clone()
            .ok_or(FsError::BadHandle)?;
        if !dir::slot_still_holds(
            &mut self.device,
            snapshot.slot_sector,
            snapshot.slot_offset,
            &snapshot.slot_name,
            snapshot.first_cluster,
        )? {
            return Err(FsError::BadHandle);
        }
        let cluster_bytes = self.cluster_bytes();
        let needed = end.div_ceil(cluster_bytes) as usize;
        let mut chain = snapshot.chain.clone();
        let mut first_cluster = snapshot.first_cluster;
        if needed > chain.len() {
            let extra = (needed - chain.len()) as u32;
            self.reserve_clusters(extra)?;
            match chain.last() {
                Some(tail) => {
                    fat::append_chain(&mut self.device, &self.boot, *tail, extra)?;
                }
                None => {
                    first_cluster = fat::allocate_chain(&mut self.device, &self.boot, extra)?;
                }
            }
            chain = fat::cluster_chain(&mut self.device, &self.boot, first_cluster)?;
        }
        let mut written = 0usize;
        let mut block = [0u8; 512];
        while written < data.len() {
            let byte = offset + written as u64;
            let cluster = chain[(byte / cluster_bytes) as usize];
            let in_cluster = (byte % cluster_bytes) as usize;
            let first_sector = self
                .boot
                .first_sector_of_cluster(cluster)
                .ok_or(FsError::Corrupt)?;
            let sector = first_sector + (in_cluster / 512) as u64;
            let sector_offset = in_cluster % 512;
            let chunk = (512 - sector_offset).min(data.len() - written);
            if sector_offset != 0 || chunk != 512 {
                self.device
                    .read_block(sector, &mut block)
                    .map_err(FsError::Io)?;
            }
            block[sector_offset..sector_offset + chunk]
                .copy_from_slice(&data[written..written + chunk]);
            self.device
                .write_block(sector, &block)
                .map_err(FsError::Io)?;
            written += chunk;
        }
        let size = snapshot.size.max(end);
        let mut name_parts = ([0u8; 8], [0u8; 3]);
        name_parts.0.copy_from_slice(&snapshot.slot_name[..8]);
        name_parts.1.copy_from_slice(&snapshot.slot_name[8..11]);
        let slot = dir::make_slot_bytes(name_parts, snapshot.attrs, first_cluster, size as u32);
        dir::write_slot_bytes(&mut self.device, snapshot.slot_sector, snapshot.slot_offset, &slot)?;
        if let Some(entry) = self.opens[handle.slot as usize].as_mut() {
            entry.first_cluster = first_cluster;
            entry.size = size;
            entry.chain = chain;
        }
        Ok(data.len())
    }

    pub fn remove(&mut self, path_bytes: &[u8]) -> Result<(), FsError> {
        let parts = path::parse(path_bytes)?;
        if parts.is_root() {
            return Err(FsError::IsADirectory);
        }
        let name = parts.last_segment().ok_or(FsError::InvalidPath)?;
        let parent = self.parent_location(&parts)?;
        let located = dir::locate_slot(&mut self.device, &self.boot, parent, name)?;
        if located.entry.is_directory {
            return Err(FsError::IsADirectory);
        }
        let freed = self.data_chain(located.entry.first_cluster)?.len() as u32;
        dir::erase_slot(&mut self.device, located.sector, located.offset)?;
        fat::free_chain(&mut self.device, &self.boot, located.entry.first_cluster)?;
        self.release_clusters(freed)
    }

    pub fn mkdir(&mut self, path_bytes: &[u8]) -> Result<(), FsError> {
        let parts = path::parse(path_bytes)?;
        if parts.is_root() {
            return Err(FsError::AlreadyExists);
        }
        let name = parts.last_segment().ok_or(FsError::InvalidPath)?;
        let folded = dir::fold_to_slot(name)?;
        let parent = self.parent_location(&parts)?;
        if dir::locate_slot(&mut self.device, &self.boot, parent, name).is_ok() {
            return Err(FsError::AlreadyExists);
        }
        let (sector, offset) = self.allocate_slot(parent)?;
        self.reserve_clusters(1)?;
        let cluster = fat::allocate_chain(&mut self.device, &self.boot, 1)?;
        let slot = dir::make_slot_bytes(folded, ATTR_DIRECTORY, cluster, 0);
        dir::write_slot_bytes(&mut self.device, sector, offset, &slot)?;
        Ok(())
    }

    pub fn rmdir(&mut self, path_bytes: &[u8]) -> Result<(), FsError> {
        let parts = path::parse(path_bytes)?;
        if parts.is_root() {
            return Err(FsError::InvalidPath);
        }
        let name = parts.last_segment().ok_or(FsError::InvalidPath)?;
        let parent = self.parent_location(&parts)?;
        let located = dir::locate_slot(&mut self.device, &self.boot, parent, name)?;
        if !located.entry.is_directory {
            return Err(FsError::NotADirectory);
        }
        let chain = self.data_chain(located.entry.first_cluster)?;
        let entries =
            dir::read_directory(&mut self.device, &self.boot, DirLocation::Chain(located.entry.first_cluster))?;
        if !entries.is_empty() {
            return Err(FsError::DirectoryNotEmpty);
        }
        dir::erase_slot(&mut self.device, located.sector, located.offset)?;
        fat::free_chain(&mut self.device, &self.boot, located.entry.first_cluster)?;
        self.release_clusters(chain.len() as u32)
    }
}

impl<D: BlockDevice> Engine for FileSystem<D> {
    type Handle = FatHandle;
    type Metadata = FatMetadata;

    fn lookup(&mut self, path_bytes: &[u8]) -> Result<Self::Metadata, FsError> {
        FileSystem::lookup(self, path_bytes)
    }

    fn open(&mut self, path_bytes: &[u8]) -> Result<Self::Handle, FsError> {
        FileSystem::open(self, path_bytes)
    }

    fn create(&mut self, path_bytes: &[u8]) -> Result<Self::Handle, FsError> {
        FileSystem::create(self, path_bytes)
    }

    fn mkdir(&mut self, path_bytes: &[u8]) -> Result<(), FsError> {
        FileSystem::mkdir(self, path_bytes)
    }

    fn rmdir(&mut self, path_bytes: &[u8]) -> Result<(), FsError> {
        FileSystem::rmdir(self, path_bytes)
    }

    fn read(
        &mut self,
        handle: Self::Handle,
        offset: u64,
        buf: &mut [u8],
    ) -> Result<usize, FsError> {
        FileSystem::read(self, handle, offset, buf)
    }

    fn write(
        &mut self,
        handle: Self::Handle,
        offset: u64,
        data: &[u8],
    ) -> Result<usize, FsError> {
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
        self.device.flush().map_err(FsError::Io)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fat::fixtures::{
        fat16_boot, fat16_entry_at, install_boot, put_slot, slot_bytes, TestDevice,
    };
    use alloc::format;
    use alloc::vec;

    fn volume_device() -> TestDevice {
        let boot = fat16_boot();
        let mut device = TestDevice::new(boot.total_sectors);
        install_boot(&mut device, &boot);
        let label = slot_bytes(b"MYLABEL ", b"   ", 0x08, 0, 0);
        let mut deleted = slot_bytes(b"GONE    ", b"TXT", 0x20, 30, 40);
        deleted[0] = 0xE5;
        let notes = slot_bytes(b"NOTES   ", b"TXT", 0x20, 2, 700);
        let games = slot_bytes(b"GAMES   ", b"   ", 0x10, 3, 0);
        let empty = slot_bytes(b"EMPTY   ", b"   ", 0x20, 0, 0);
        put_slot(&mut device, 17, 0, &label);
        put_slot(&mut device, 17, 1, &deleted);
        put_slot(&mut device, 17, 2, &notes);
        put_slot(&mut device, 17, 3, &games);
        put_slot(&mut device, 17, 4, &empty);
        let snake = slot_bytes(b"SNAKE   ", b"EXE", 0x20, 4, 3);
        let sub = slot_bytes(b"SUB     ", b"   ", 0x10, 5, 0);
        put_slot(&mut device, 50, 0, &snake);
        put_slot(&mut device, 50, 1, &sub);
        let deep = slot_bytes(b"DEEP    ", b"BIN", 0x20, 6, 5);
        put_slot(&mut device, 52, 0, &deep);
        fill_from(&mut device, 49, 0xAA);
        fill_from(&mut device, 56, 0x55);
        device.bytes[51 * 512..51 * 512 + 3].copy_from_slice(&[1, 2, 3]);
        device.bytes[53 * 512..53 * 512 + 5].copy_from_slice(&[10, 11, 12, 13, 14]);
        let (sector, offset) = fat16_entry_at(2);
        device.put16(sector, offset, 9);
        let (sector, offset) = fat16_entry_at(9);
        device.put16(sector, offset, 0xFFF8);
        for cluster in [3u32, 4, 5, 6] {
            let (sector, offset) = fat16_entry_at(cluster);
            device.put16(sector, offset, 0xFFF8);
        }
        device
    }

    fn fill_from(device: &mut TestDevice, sector: u64, start_byte: u8) {
        let at = sector as usize * 512;
        for (index, byte) in device.bytes[at..at + 512].iter_mut().enumerate() {
            *byte = start_byte.wrapping_add(index as u8);
        }
    }

    fn engine() -> FileSystem<TestDevice> {
        FileSystem::new(volume_device()).expect("volume opens")
    }

    fn dir_item(kind: Kind, name: &[u8]) -> DirItem {
        DirItem {
            kind,
            name: name.to_vec(),
        }
    }

    #[test]
    fn new_accepts_a_valid_fat16_volume() {
        assert!(FileSystem::new(volume_device()).is_ok());
    }

    #[test]
    fn lookup_root_is_a_directory() {
        let mut fs = engine();
        let meta = Engine::lookup(&mut fs, b"/").expect("root");
        assert_eq!(meta.kind(), Kind::Directory);
    }

    #[test]
    fn lookup_file_metadata_is_case_insensitive() {
        let mut fs = engine();
        for path in ["/notes.txt", "/NOTES.TXT", "/NoTeS.TxT"] {
            let meta = Engine::lookup(&mut fs, path.as_bytes()).expect("found");
            assert_eq!((meta.kind(), meta.size()), (Kind::Regular, 700), "{path}");
        }
    }

    #[test]
    fn lookup_nested_directories_and_files() {
        let mut fs = engine();
        let games = Engine::lookup(&mut fs, b"/games").expect("games");
        assert_eq!(games.kind(), Kind::Directory);
        let snake = Engine::lookup(&mut fs, b"/games/snake.exe").expect("snake");
        assert_eq!((snake.kind(), snake.size()), (Kind::Regular, 3));
        let deep = Engine::lookup(&mut fs, b"/games/sub/deep.bin").expect("deep");
        assert_eq!((deep.kind(), deep.size()), (Kind::Regular, 5));
    }

    #[test]
    fn lookup_reports_missing_and_file_as_parent() {
        let mut fs = engine();
        assert_eq!(Engine::lookup(&mut fs, b"/nope"), Err(FsError::NotFound));
        assert_eq!(Engine::lookup(&mut fs, b"/games/nope"), Err(FsError::NotFound));
        assert_eq!(
            Engine::lookup(&mut fs, b"/notes.txt/inner"),
            Err(FsError::NotADirectory)
        );
        assert_eq!(
            Engine::lookup(&mut fs, b"/games/sub/deep.bin/x"),
            Err(FsError::NotADirectory)
        );
    }

    #[test]
    fn lookup_skips_deleted_slots_and_volume_labels() {
        let mut fs = engine();
        assert_eq!(Engine::lookup(&mut fs, b"/gone.txt"), Err(FsError::NotFound));
        assert_eq!(Engine::lookup(&mut fs, b"/mylabel"), Err(FsError::NotFound));
    }

    #[test]
    fn open_rejects_directories() {
        let mut fs = engine();
        assert_eq!(Engine::open(&mut fs, b"/"), Err(FsError::IsADirectory));
        assert_eq!(Engine::open(&mut fs, b"/games"), Err(FsError::IsADirectory));
    }

    #[test]
    fn open_read_stat_and_close_lifecycle() {
        let mut fs = engine();
        let handle = Engine::open(&mut fs, b"/notes.txt").expect("open");
        let meta = Engine::stat(&mut fs, handle).expect("stat");
        assert_eq!((meta.kind(), meta.size()), (Kind::Regular, 700));
        let mut buf = [0u8; 700];
        assert_eq!(Engine::read(&mut fs, handle, 0, &mut buf).expect("read"), 700);
        for (index, byte) in buf[..512].iter().enumerate() {
            assert_eq!(*byte, 0xAAu8.wrapping_add(index as u8), "byte {index}");
        }
        for (index, byte) in buf[512..].iter().enumerate() {
            assert_eq!(*byte, 0x55u8.wrapping_add(index as u8), "byte {index}");
        }
        let mut tail = [0u8; 200];
        assert_eq!(Engine::read(&mut fs, handle, 600, &mut tail).expect("tail"), 100);
        assert_eq!(tail[0], 0x55u8.wrapping_add(88));
        assert_eq!(Engine::read(&mut fs, handle, 700, &mut tail).expect("eof"), 0);
        assert_eq!(Engine::close(&mut fs, handle).expect("close"), ());
        assert_eq!(Engine::close(&mut fs, handle), Err(FsError::BadHandle));
        assert_eq!(Engine::read(&mut fs, handle, 0, &mut tail), Err(FsError::BadHandle));
    }

    #[test]
    fn stale_handles_lose_after_slot_reuse() {
        let mut fs = engine();
        let first = Engine::open(&mut fs, b"/notes.txt").expect("first");
        Engine::close(&mut fs, first).expect("close");
        let second = Engine::open(&mut fs, b"/games/snake.exe").expect("second");
        assert_eq!(first.slot, second.slot);
        let mut buf = [0u8; 16];
        assert_eq!(Engine::read(&mut fs, first, 0, &mut buf), Err(FsError::BadHandle));
        assert_eq!(Engine::read(&mut fs, second, 0, &mut buf).expect("fresh"), 3);
    }

    #[test]
    fn empty_files_open_and_read_zero() {
        let mut fs = engine();
        let handle = Engine::open(&mut fs, b"/empty").expect("open empty");
        assert_eq!(Engine::read(&mut fs, handle, 0, &mut [0u8; 8]).expect("read"), 0);
    }

    #[test]
    fn readdir_lists_kinds_in_slot_order() {
        let mut fs = engine();
        assert_eq!(
            Engine::readdir(&mut fs, b"/").expect("root listing"),
            vec![
                dir_item(Kind::Regular, b"notes.txt"),
                dir_item(Kind::Directory, b"games"),
                dir_item(Kind::Regular, b"empty"),
            ]
        );
        assert_eq!(
            Engine::readdir(&mut fs, b"/games").expect("games listing"),
            vec![
                dir_item(Kind::Regular, b"snake.exe"),
                dir_item(Kind::Directory, b"sub"),
            ]
        );
        assert_eq!(
            Engine::readdir(&mut fs, b"/games/sub/").expect("sub listing"),
            vec![dir_item(Kind::Regular, b"deep.bin")]
        );
    }

    #[test]
    fn readdir_rejects_files_and_missing_paths() {
        let mut fs = engine();
        assert_eq!(Engine::readdir(&mut fs, b"/notes.txt"), Err(FsError::NotADirectory));
        assert_eq!(Engine::readdir(&mut fs, b"/missing"), Err(FsError::NotFound));
    }

    #[test]
    fn create_write_read_round_trip() {
        let mut fs = engine();
        let handle = Engine::create(&mut fs, b"/fresh.txt").expect("create");
        assert_eq!(Engine::write(&mut fs, handle, 0, b"hello fat").expect("write"), 9);
        Engine::close(&mut fs, handle).expect("close");
        let meta = Engine::lookup(&mut fs, b"/fresh.txt").expect("lookup");
        assert_eq!((meta.kind(), meta.size()), (Kind::Regular, 9));
        let reopened = Engine::open(&mut fs, b"/fresh.txt").expect("reopen");
        let mut buf = [0u8; 16];
        assert_eq!(Engine::read(&mut fs, reopened, 0, &mut buf).expect("read"), 9);
        assert_eq!(&buf[..9], b"hello fat");
    }

    #[test]
    fn create_refuses_collisions_bad_names_and_missing_parents() {
        let mut fs = engine();
        assert_eq!(Engine::create(&mut fs, b"/notes.txt"), Err(FsError::AlreadyExists));
        assert_eq!(Engine::create(&mut fs, b"/games"), Err(FsError::AlreadyExists));
        assert_eq!(Engine::create(&mut fs, b"/a.b.txt"), Err(FsError::InvalidName));
        assert_eq!(Engine::create(&mut fs, b"/toolongstem"), Err(FsError::InvalidName));
        let reused = Engine::create(&mut fs, b"/gone.txt").expect("deleted name is reusable");
        Engine::close(&mut fs, reused).expect("close");
        assert_eq!(Engine::create(&mut fs, b"/gone.txt"), Err(FsError::AlreadyExists));
        assert_eq!(Engine::create(&mut fs, b"/missing/parent"), Err(FsError::NotFound));
        assert_eq!(Engine::create(&mut fs, b"/notes.txt/below"), Err(FsError::NotADirectory));
        assert_eq!(Engine::create(&mut fs, b"/"), Err(FsError::IsADirectory));
    }

    #[test]
    fn write_grows_across_clusters_and_zero_fills_gaps() {
        let mut fs = engine();
        let handle = Engine::create(&mut fs, b"/big").expect("create");
        Engine::write(&mut fs, handle, 0, b"head").expect("write head");
        let payload = [0x5Au8; 600];
        Engine::write(&mut fs, handle, 600, &payload).expect("write far");
        assert_eq!(Engine::stat(&mut fs, handle).expect("stat").size(), 1_200);
        let mut gap = [0xFFu8; 4];
        assert_eq!(Engine::read(&mut fs, handle, 500, &mut gap).expect("gap"), 4);
        assert_eq!(gap, [0u8; 4]);
        let mut tail = [0u8; 600];
        assert_eq!(Engine::read(&mut fs, handle, 600, &mut tail).expect("tail"), 600);
        assert!(tail.iter().all(|byte| *byte == 0x5A));
    }

    #[test]
    fn oversized_writes_are_refused_before_any_touch() {
        let mut fs = engine();
        let handle = Engine::create(&mut fs, b"/huge").expect("create");
        assert_eq!(
            Engine::write(&mut fs, handle, u64::from(u32::MAX) - 2, b"abcd"),
            Err(FsError::FileTooLarge)
        );
        let mut buf = [0u8; 8];
        assert_eq!(Engine::read(&mut fs, handle, 0, &mut buf).expect("still empty"), 0);
    }

    #[test]
    fn written_files_survive_a_remount() {
        let device = {
            let mut fs = engine();
            let handle = Engine::create(&mut fs, b"/persist.dat").expect("create");
            Engine::write(&mut fs, handle, 0, b"keep me").expect("write");
            Engine::close(&mut fs, handle).expect("close");
            fs.device
        };
        let mut fs = FileSystem::new(device).expect("remount");
        let handle = Engine::open(&mut fs, b"/persist.dat").expect("open after remount");
        let mut buf = [0u8; 7];
        assert_eq!(Engine::read(&mut fs, handle, 0, &mut buf).expect("read"), 7);
        assert_eq!(&buf, b"keep me");
    }

    #[test]
    fn remove_erases_and_reclaims_clusters() {
        let mut fs = engine();
        let free_before = fat::count_free_clusters(&mut fs.device, &fs.boot).expect("count");
        let handle = Engine::create(&mut fs, b"/temp.bin").expect("create");
        Engine::write(&mut fs, handle, 0, b"1234567890").expect("write");
        Engine::close(&mut fs, handle).expect("close");
        assert_eq!(Engine::remove(&mut fs, b"/temp.bin").expect("remove"), ());
        assert_eq!(Engine::lookup(&mut fs, b"/temp.bin"), Err(FsError::NotFound));
        assert_eq!(
            fat::count_free_clusters(&mut fs.device, &fs.boot).expect("recount"),
            free_before
        );
        assert_eq!(Engine::readdir(&mut fs, b"/").expect("listing").len(), 3);
        assert_eq!(Engine::remove(&mut fs, b"/games"), Err(FsError::IsADirectory));
        assert_eq!(Engine::remove(&mut fs, b"/missing"), Err(FsError::NotFound));
        assert_eq!(Engine::remove(&mut fs, b"/"), Err(FsError::IsADirectory));
    }

    #[test]
    fn a_removed_slot_fails_writes_through_open_handles() {
        let mut fs = engine();
        let handle = Engine::open(&mut fs, b"/notes.txt").expect("open");
        Engine::remove(&mut fs, b"/notes.txt").expect("remove underneath");
        assert_eq!(Engine::write(&mut fs, handle, 0, b"x"), Err(FsError::BadHandle));
    }

    #[test]
    fn mkdir_lists_and_accepts_children() {
        let mut fs = engine();
        Engine::mkdir(&mut fs, b"/reports").expect("mkdir");
        assert_eq!(Engine::readdir(&mut fs, b"/reports").expect("empty dir"), vec![]);
        let child = Engine::create(&mut fs, b"/reports/q1.txt").expect("child create");
        Engine::write(&mut fs, child, 0, b"quarter").expect("write child");
        Engine::close(&mut fs, child).expect("close child");
        assert_eq!(
            Engine::readdir(&mut fs, b"/reports").expect("listing"),
            vec![dir_item(Kind::Regular, b"q1.txt")]
        );
        assert_eq!(Engine::lookup(&mut fs, b"/reports/q1.txt").expect("lookup").size(), 7);
        assert_eq!(Engine::mkdir(&mut fs, b"/reports"), Err(FsError::AlreadyExists));
        assert_eq!(Engine::mkdir(&mut fs, b"/notes.txt/x"), Err(FsError::NotADirectory));
        assert_eq!(Engine::mkdir(&mut fs, b"/bad name"), Err(FsError::InvalidName));
    }

    #[test]
    fn rmdir_rules() {
        let mut fs = engine();
        assert_eq!(Engine::rmdir(&mut fs, b"/games"), Err(FsError::DirectoryNotEmpty));
        assert_eq!(Engine::rmdir(&mut fs, b"/notes.txt"), Err(FsError::NotADirectory));
        assert_eq!(Engine::rmdir(&mut fs, b"/missing"), Err(FsError::NotFound));
        assert_eq!(Engine::rmdir(&mut fs, b"/"), Err(FsError::InvalidPath));
        Engine::mkdir(&mut fs, b"/brief").expect("mkdir");
        assert_eq!(Engine::rmdir(&mut fs, b"/brief").expect("rmdir"), ());
        assert_eq!(Engine::lookup(&mut fs, b"/brief"), Err(FsError::NotFound));
    }

    #[test]
    fn a_full_root_region_answers_no_space() {
        let mut fs = engine();
        for index in 0..508u32 {
            let name = format!("/F{index:03}");
            Engine::create(&mut fs, name.as_bytes()).expect("room for 508: one deleted slot plus 507 empty");
        }
        assert_eq!(Engine::create(&mut fs, b"/overflow"), Err(FsError::NoSpace));
        assert_eq!(Engine::readdir(&mut fs, b"/").expect("listing").len(), 511);
    }

    #[test]
    fn fat32_root_grows_by_chain_extension_and_keeps_fsinfo() {
        use crate::boot::RootLocation;
        let boot = BootSector {
            bytes_per_sector: 512,
            sectors_per_cluster: 1,
            fat_count: 2,
            reserved_sectors: 32,
            fat_sectors: 512,
            total_sectors: 66_589,
            first_data_sector: 1_064,
            cluster_count: 65_525,
            fat_type: FatType::Fat32,
            root: RootLocation::Cluster(2),
            fsinfo_sector: 1,
        };
        let mut device = TestDevice::new(boot.total_sectors);
        install_boot(&mut device, &boot);
        let mark_root = |device: &mut TestDevice, cluster: u32, value: u32| {
            for copy in 0..2usize {
                let at = 32 * 512 + copy * 512 * 512 + (cluster as usize) * 4;
                device.bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
            }
        };
        mark_root(&mut device, 2, 0x0FFF_FFF8);
        let mut fs = FileSystem::new(device).expect("fat32 opens");
        for index in 0..20u32 {
            let name = format!("/R{index:02}");
            Engine::create(&mut fs, name.as_bytes()).expect("create");
        }
        assert_eq!(Engine::readdir(&mut fs, b"/").expect("listing").len(), 20);
        let counted = fat::count_free_clusters(&mut fs.device, &fs.boot).expect("count");
        assert_eq!(fs.free_clusters, Some(counted));
        let at = 512 + 0x1E8;
        assert_eq!(
            u32::from_le_bytes([
                fs.device.bytes[at],
                fs.device.bytes[at + 1],
                fs.device.bytes[at + 2],
                fs.device.bytes[at + 3]
            ]),
            counted
        );
    }

    #[test]
    fn sync_is_idempotent_on_a_read_only_volume() {
        let mut fs = engine();
        Engine::sync(&mut fs).expect("first sync");
        Engine::sync(&mut fs).expect("second sync");
    }

    #[test]
    fn sync_flushes_the_device() {
        let mut fs = engine();
        Engine::sync(&mut fs).expect("sync");
        Engine::sync(&mut fs).expect("sync again");
        assert_eq!(fs.device.flushes, 2);
    }

    #[test]
    fn rejects_paths_by_the_shared_model() {
        let mut fs = engine();
        assert_eq!(Engine::lookup(&mut fs, b"relative"), Err(FsError::InvalidPath));
        let long_name = alloc::format!("/{}", "k".repeat(256));
        assert_eq!(
            Engine::lookup(&mut fs, long_name.as_bytes()),
            Err(FsError::NameTooLong)
        );
    }

    #[test]
    fn fat12_volumes_are_unsupported() {
        let mut device = TestDevice::new(4_131);
        device.bytes[0x0B..0x0D].copy_from_slice(&512u16.to_le_bytes());
        device.bytes[0x0D] = 1;
        device.bytes[0x0E..0x10].copy_from_slice(&1u16.to_le_bytes());
        device.bytes[0x10] = 2;
        device.bytes[0x11..0x13].copy_from_slice(&224u16.to_le_bytes());
        device.bytes[0x16..0x18].copy_from_slice(&16u16.to_le_bytes());
        device.bytes[0x20..0x24].copy_from_slice(&4_131u32.to_le_bytes());
        device.bytes[0x36..0x3E].copy_from_slice(b"FAT12   ");
        device.bytes[0x1FE] = 0x55;
        device.bytes[0x1FF] = 0xAA;
        assert!(matches!(FileSystem::new(device), Err(FsError::Unsupported)));
    }

    #[test]
    fn non_fat_media_is_unsupported() {
        let device = TestDevice::new(65_536);
        assert!(matches!(FileSystem::new(device), Err(FsError::Unsupported)));
    }

    #[test]
    fn device_smaller_than_the_volume_is_reported() {
        let boot = fat16_boot();
        let mut device = TestDevice::new(100);
        install_boot(&mut device, &boot);
        assert!(matches!(FileSystem::new(device), Err(FsError::DeviceTooSmall)));
    }
}
