use fs_core::engine::Engine;
use fs_core::engine::MetadataView;
use fs_core::error::FsError;
use rosx_boot_common::crc32::crc32;
use rosx_boot_common::memnorm::place_payload;
use rosx_boot_common::serial::Serial;
use rosx_boot_common::DEBUG_EXIT_FAILURE;
use rosx_boot_common::debug_exit;

use crate::blk::BiosBlockDevice;

pub type BootVolume<'a> = rxfat::FileSystem<BiosBlockDevice<'a>>;

const INITRD_PATH: &[u8] = b"/INITRD.BIN";
const READ_WINDOW: usize = 16 * 512;

pub fn open<'a>(console: &Serial, device: BiosBlockDevice<'a>) -> BootVolume<'a> {
    rxfat::FileSystem::new(device).unwrap_or_else(|error| refuse_engine(console, error))
}

pub fn place_initrd(
    console: &Serial,
    volume: &mut BootVolume,
    kernel_end: u64,
    ceiling: u64,
) -> u64 {
    let metadata =
        Engine::lookup(volume, INITRD_PATH).unwrap_or_else(|error| refuse_engine(console, error));
    let size = usize::try_from(metadata.size()).unwrap_or_else(|_| refuse_memory(console));
    let placement =
        place_payload(kernel_end, metadata.size(), ceiling).unwrap_or_else(|| refuse_memory(console));
    let handle =
        Engine::open(volume, INITRD_PATH).unwrap_or_else(|error| refuse_engine(console, error));
    let staging =
        unsafe { core::slice::from_raw_parts_mut(placement.address as *mut u8, size) };
    let mut loaded = 0usize;
    while loaded < size {
        let span = (size - loaded).min(READ_WINDOW);
        let count = Engine::read(
            volume,
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
    Engine::close(volume, handle).unwrap_or_else(|error| refuse_engine(console, error));
    let checksum = crc32(staging);
    console.write_str("[INITRD] addr=");
    console.write_hex(placement.address, 8);
    console.write_str(" size=");
    console.write_dec(size as u64);
    console.write_str(" crc32=");
    console.write_hex(u64::from(checksum), 8);
    console.write_line("");
    placement.end
}

fn refuse_memory(console: &Serial) -> ! {
    console.write_line("[ERR] mem");
    debug_exit(DEBUG_EXIT_FAILURE)
}

pub(crate) fn refuse_engine(console: &Serial, error: FsError) -> ! {
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
