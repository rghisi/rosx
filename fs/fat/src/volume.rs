use crate::boot::{BootSector, FatType};
use crate::device::BlockDevice;
use crate::dir::{self, DirEntry, DirLocation};
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
}

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
        let resolved = self.resolve(&parts)?;
        if resolved.is_directory {
            return Err(FsError::IsADirectory);
        }
        let generation = self.next_generation;
        self.next_generation = self.next_generation.wrapping_add(1);
        let entry = OpenEntry {
            generation,
            first_cluster: resolved.first_cluster,
            size: resolved.size,
        };
        let slot = match self.opens.iter().position(|slot| slot.is_none()) {
            Some(slot) => slot,
            None => {
                self.opens.push(None);
                self.opens.len() - 1
            }
        };
        self.opens[slot] = Some(entry);
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

    fn create(&mut self, _path_bytes: &[u8]) -> Result<Self::Handle, FsError> {
        Err(FsError::Unsupported)
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
    ) -> Result<usize, FsError> {
        FileSystem::read(self, handle, offset, buf)
    }

    fn write(
        &mut self,
        _handle: Self::Handle,
        _offset: u64,
        _data: &[u8],
    ) -> Result<usize, FsError> {
        Err(FsError::Unsupported)
    }

    fn stat(&mut self, handle: Self::Handle) -> Result<Self::Metadata, FsError> {
        FileSystem::stat(self, handle)
    }

    fn close(&mut self, handle: Self::Handle) -> Result<(), FsError> {
        FileSystem::close(self, handle)
    }

    fn remove(&mut self, _path_bytes: &[u8]) -> Result<(), FsError> {
        Err(FsError::Unsupported)
    }

    fn readdir(&mut self, path_bytes: &[u8]) -> Result<Vec<DirItem>, FsError> {
        FileSystem::readdir(self, path_bytes)
    }

    fn sync(&mut self) -> Result<(), FsError> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fat::fixtures::{
        fat16_boot, fat16_entry_at, install_boot, put_slot, slot_bytes, TestDevice,
    };
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
    fn mutators_answer_unsupported() {
        let mut fs = engine();
        assert_eq!(Engine::create(&mut fs, b"/new.txt"), Err(FsError::Unsupported));
        assert_eq!(Engine::mkdir(&mut fs, b"/newdir"), Err(FsError::Unsupported));
        assert_eq!(Engine::rmdir(&mut fs, b"/games"), Err(FsError::Unsupported));
        assert_eq!(Engine::remove(&mut fs, b"/notes.txt"), Err(FsError::Unsupported));
        let handle = Engine::open(&mut fs, b"/notes.txt").expect("open");
        assert_eq!(
            Engine::write(&mut fs, handle, 0, b"data"),
            Err(FsError::Unsupported)
        );
    }

    #[test]
    fn sync_is_idempotent_on_a_read_only_volume() {
        let mut fs = engine();
        Engine::sync(&mut fs).expect("first sync");
        Engine::sync(&mut fs).expect("second sync");
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
