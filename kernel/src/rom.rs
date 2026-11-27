use core::sync::atomic::{AtomicU8, AtomicUsize, Ordering};

use crate::once::Once;
use crate::task::GroupId;
use system::error::ErrorCode;
use system::rom::RomError;
use system::rom::RomImage;
use system::rom::RomRef;

static ROM_IMAGE: Once<&'static [u8]> = Once::new();
static LIFECYCLE: Lifecycle = Lifecycle::new();

struct Lifecycle {
    state: AtomicU8,
    owner: AtomicUsize,
}

impl Lifecycle {
    const HELD: u8 = 0;
    const MAPPED: u8 = 1;
    const RELEASED: u8 = 2;

    const fn new() -> Self {
        Lifecycle {
            state: AtomicU8::new(Self::HELD),
            owner: AtomicUsize::new(0),
        }
    }

    fn is_released(&self) -> bool {
        self.state.load(Ordering::SeqCst) == Self::RELEASED
    }

    fn map(&self, caller: usize) -> Result<(), ErrorCode> {
        match self
            .state
            .compare_exchange(Self::HELD, Self::MAPPED, Ordering::SeqCst, Ordering::SeqCst)
        {
            Ok(_) => {
                self.owner.store(caller, Ordering::SeqCst);
                Ok(())
            }
            Err(Self::MAPPED) if self.owner.load(Ordering::SeqCst) == caller => Ok(()),
            Err(Self::MAPPED) => Err(ErrorCode::RomNotOwner),
            Err(_) => Err(ErrorCode::RomReleased),
        }
    }

    fn claim_release(&self, caller: usize) -> Result<(), ErrorCode> {
        let state = self.state.load(Ordering::SeqCst);
        if state == Self::HELD {
            return Err(ErrorCode::RomNotOwner);
        }
        if state == Self::RELEASED {
            return Err(ErrorCode::RomAlreadyReleased);
        }
        if self.owner.load(Ordering::SeqCst) != caller {
            return Err(ErrorCode::RomNotOwner);
        }
        self.state
            .compare_exchange(Self::MAPPED, Self::RELEASED, Ordering::SeqCst, Ordering::SeqCst)
            .map(|_| ())
            .map_err(|_| ErrorCode::RomAlreadyReleased)
    }
}

pub fn init(image: &'static [u8]) {
    ROM_IMAGE.call_once(|| image);
}

pub fn image() -> &'static [u8] {
    if LIFECYCLE.is_released() {
        return &[];
    }
    ROM_IMAGE.get().copied().unwrap_or(&[])
}

pub fn validate(image: &[u8]) -> Result<usize, RomError> {
    Ok(RomImage::parse(image)?.files().count())
}

pub fn program(name: &str) -> Option<&'static [u8]> {
    located(image(), name)
}

fn located<'a>(image: &'a [u8], name: &str) -> Option<&'a [u8]> {
    let entry = RomImage::parse(image)
        .ok()?
        .files()
        .find(|file| file.name == name)?;
    image.get(entry.offset..entry.offset + entry.len)
}

pub fn map(caller: GroupId) -> Result<RomRef, ErrorCode> {
    LIFECYCLE.map(caller.value())?;
    let image = image();
    Ok(RomRef {
        start: image.as_ptr() as u64,
        len: image.len() as u64,
    })
}

pub fn release(caller: GroupId) -> Result<usize, ErrorCode> {
    LIFECYCLE.claim_release(caller.value())?;
    let address = ROM_IMAGE.get().copied().unwrap_or(&[]).as_ptr() as usize;
    let bytes = crate::memory::memory_manager::MEMORY_MANAGER.release_held();
    crate::kprintln!("[ROM] released addr=0x{:x} bytes={}", address, bytes);
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use system::rom::encode;

    #[test]
    fn located_finds_the_named_program_bytes() {
        let image = encode(&[("init", b"elf-init".as_slice()), ("shell", b"elf-shell".as_slice())]);
        assert_eq!(located(&image, "init"), Some(b"elf-init".as_slice()));
        assert_eq!(located(&image, "shell"), Some(b"elf-shell".as_slice()));
    }

    #[test]
    fn located_misses_a_name_the_rom_does_not_carry() {
        let image = encode(&[("init", b"elf".as_slice())]);
        assert_eq!(located(&image, "nope"), None);
    }

    #[test]
    fn located_refuses_a_malformed_image() {
        assert_eq!(located(b"ROSXROM1not-a-rom-image", "init"), None);
        assert_eq!(located(&encode(&[("init", b"elf".as_slice())])[..6], "init"), None);
    }

    #[test]
    fn located_refuses_an_empty_image() {
        assert_eq!(located(&[], "init"), None);
    }

    #[test]
    fn located_refuses_an_image_without_init() {
        let image = encode(&[("atad", b"a".as_slice()), ("shell", b"s".as_slice())]);
        assert_eq!(located(&image, "init"), None);
    }

    #[test]
    fn validate_counts_the_files_of_a_wellformed_image() {
        let image = encode(&[("init", b"i".as_slice()), ("shell", b"s".as_slice())]);
        assert_eq!(validate(&image), Ok(2));
        assert_eq!(validate(&encode(&[])), Ok(0));
    }

    #[test]
    fn validate_names_what_is_wrong_with_the_image() {
        assert_eq!(validate(&[]), Err(RomError::TooShort));
        assert_eq!(validate(b"NOTAROM!1234"), Err(RomError::BadMagic));
    }

    #[test]
    fn init_registers_the_image_for_program() {
        static IMAGE: &[u8] = b"kernel rom image";
        init(IMAGE);
        assert_eq!(image(), IMAGE);
        assert_eq!(located(image(), "nope"), None);
    }

    #[test]
    fn a_fresh_lifecycle_maps_for_exactly_one_group() {
        let lifecycle = Lifecycle::new();
        assert_eq!(lifecycle.map(7), Ok(()));
        assert_eq!(lifecycle.map(7), Ok(()), "the owner may map again");
        assert_eq!(lifecycle.map(8), Err(ErrorCode::RomNotOwner));
    }

    #[test]
    fn release_requires_the_mapping_owner() {
        let lifecycle = Lifecycle::new();
        assert_eq!(lifecycle.claim_release(7), Err(ErrorCode::RomNotOwner), "nothing is mapped yet");
        lifecycle.map(7).unwrap();
        assert_eq!(lifecycle.claim_release(8), Err(ErrorCode::RomNotOwner));
        assert_eq!(lifecycle.claim_release(7), Ok(()));
        assert_eq!(lifecycle.claim_release(7), Err(ErrorCode::RomAlreadyReleased));
    }

    #[test]
    fn a_released_lifecycle_refuses_to_map_again() {
        let lifecycle = Lifecycle::new();
        lifecycle.map(7).unwrap();
        lifecycle.claim_release(7).unwrap();
        assert_eq!(lifecycle.map(9), Err(ErrorCode::RomReleased));
        assert!(lifecycle.is_released());
    }
}
