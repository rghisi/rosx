use fs_core::engine::Engine;
use fs_core::engine::MetadataView;
use fs_core::error::FsError;
use rosx_boot_common::crc32::crc32;
use rosx_boot_common::serial::Serial;
use rosx_boot_common::DEBUG_EXIT_FAILURE;
use rosx_boot_common::debug_exit;

use crate::blk::BiosBlockDevice;

const INITRD_PATH: &[u8] = b"/INITRD.BIN";
const INITRD_STAGING: usize = 0x0010_0000;
const INITRD_CAPACITY: usize = 64 * 1024;
const READ_WINDOW: usize = 16 * 512;

pub fn load_initrd(console: &Serial, device: BiosBlockDevice) {
    let mut volume =
        rxfat::FileSystem::new(device).unwrap_or_else(|error| refuse_engine(console, error));
    let metadata =
        Engine::lookup(&mut volume, INITRD_PATH).unwrap_or_else(|error| refuse_engine(console, error));
    let size = match usize::try_from(metadata.size()) {
        Ok(size) if size <= INITRD_CAPACITY => size,
        _ => refuse_kind(console, "too-large"),
    };
    let handle =
        Engine::open(&mut volume, INITRD_PATH).unwrap_or_else(|error| refuse_engine(console, error));
    let staging = unsafe { core::slice::from_raw_parts_mut(INITRD_STAGING as *mut u8, size) };
    let mut loaded = 0usize;
    while loaded < size {
        let span = (size - loaded).min(READ_WINDOW);
        let count = Engine::read(
            &mut volume,
            handle,
            loaded as u64,
            &mut staging[loaded..loaded + span],
        )
        .unwrap_or_else(|error| refuse_engine(console, error));
        if count == 0 {
            refuse_kind(console, "short-read");
        }
        loaded += count;
    }
    Engine::close(&mut volume, handle).unwrap_or_else(|error| refuse_engine(console, error));
    let checksum = crc32(staging);
    console.write_str("[FAT] initrd addr=");
    console.write_hex(INITRD_STAGING as u64, 8);
    console.write_str(" size=");
    console.write_dec(size as u64);
    console.write_str(" crc32=");
    console.write_hex(u64::from(checksum), 8);
    console.write_line("");
}

fn refuse_engine(console: &Serial, error: FsError) -> ! {
    console.write_str("[FAT-ERR] ");
    console.write_line(kind_name(error));
    debug_exit(DEBUG_EXIT_FAILURE)
}

fn refuse_kind(console: &Serial, kind: &str) -> ! {
    console.write_str("[FAT-ERR] ");
    console.write_line(kind);
    debug_exit(DEBUG_EXIT_FAILURE)
}

fn kind_name(error: FsError) -> &'static str {
    match error {
        FsError::Io(_) => "io",
        FsError::Corrupt => "corrupt",
        FsError::DeviceTooSmall => "device-too-small",
        FsError::VolumeTooLarge => "volume-too-large",
        FsError::NameTooLong => "name-too-long",
        FsError::PathTooLong => "path-too-long",
        FsError::InvalidName => "invalid-name",
        FsError::InvalidPath => "invalid-path",
        FsError::DepthExceeded => "depth-exceeded",
        FsError::NotFound => "not-found",
        FsError::AlreadyExists => "already-exists",
        FsError::NotADirectory => "not-a-directory",
        FsError::IsADirectory => "is-a-directory",
        FsError::DirectoryNotEmpty => "directory-not-empty",
        FsError::NoSpace => "no-space",
        FsError::NoInodeSpace => "no-inode-space",
        FsError::FileTooLarge => "file-too-large",
        FsError::BadHandle => "bad-handle",
        FsError::TooManyOpen => "too-many-open",
        FsError::Unsupported => "unsupported",
    }
}
