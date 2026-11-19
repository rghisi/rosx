use fs_core::engine::Engine;
use fs_core::engine::MetadataView;
use fs_core::error::FsError;
use rosx_boot_common::crc32::crc32;
use rosx_boot_common::memnorm::place_payload;
use rosx_boot_common::DEBUG_EXIT_FAILURE;
use rosx_boot_common::debug_exit;

use crate::blk::BiosBlockDevice;
use crate::console::BroadcastConsole;

pub type BootVolume<'a> = rxfat::FileSystem<BiosBlockDevice<'a>>;

const INITRD_PATH: &[u8] = b"/INITRD.BIN";
const READ_WINDOW: usize = 16 * 512;

pub fn open<'a>(console: &BroadcastConsole<'_>, device: BiosBlockDevice<'a>) -> BootVolume<'a> {
    rxfat::FileSystem::new(device).unwrap_or_else(|error| refuse_engine(console, error))
}

pub struct InitrdPlacement {
    pub address: u64,
    pub size: u64,
    pub end: u64,
}

pub fn place_initrd(
    console: &BroadcastConsole<'_>,
    volume: &mut BootVolume,
    kernel_end: u64,
    ceiling: u64,
) -> InitrdPlacement {
    let metadata = match Engine::lookup(volume, INITRD_PATH) {
        Ok(metadata) => metadata,
        Err(FsError::NotFound) => {
            return report(
                console,
                InitrdPlacement {
                    address: 0,
                    size: 0,
                    end: kernel_end,
                },
                crc32(&[]),
            );
        }
        Err(error) => refuse_engine(console, error),
    };
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
    report(
        console,
        InitrdPlacement {
            address: placement.address,
            size: metadata.size(),
            end: placement.end,
        },
        crc32(staging),
    )
}

fn report(console: &BroadcastConsole<'_>, payload: InitrdPlacement, checksum: u32) -> InitrdPlacement {
    console.write_str("[INITRD] addr=");
    console.write_hex(payload.address, 8);
    console.write_str(" size=");
    console.write_dec(payload.size);
    console.write_str(" crc32=");
    console.write_hex(u64::from(checksum), 8);
    console.write_line("");
    payload
}

fn refuse_memory(console: &BroadcastConsole<'_>) -> ! {
    console.write_line("[ERR] mem");
    debug_exit(DEBUG_EXIT_FAILURE)
}

pub(crate) fn refuse_engine(console: &BroadcastConsole<'_>, error: FsError) -> ! {
    console.write_str("[FAT-ERR] ");
    console.write_line(kind_name(error));
    debug_exit(DEBUG_EXIT_FAILURE)
}

fn refuse_kind(console: &BroadcastConsole<'_>, kind: &str) -> ! {
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
