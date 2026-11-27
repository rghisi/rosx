use crate::error::{RomMapError, RomReleaseError};
use crate::syscall::Syscall;
use system::rom::RomImage;
use system::rom::RomRef;

pub struct RomView {
    start: *const u8,
    len: usize,
}

impl RomView {
    pub fn map() -> Result<RomView, RomMapError> {
        let mut reference = RomRef { start: 0, len: 0 };
        Syscall::rom_map(&mut reference)?;
        Ok(RomView {
            start: reference.start as *const u8,
            len: reference.len as usize,
        })
    }

    /// The mapped ROM bytes, borrowed for the view's whole life.
    ///
    /// The kernel keeps the image's pages held until `release` consumes
    /// the view, so this pointer stays readable for any borrow of
    /// `self`. See `docs/rationale/rom-map-release.md`.
    pub fn image(&self) -> &[u8] {
        unsafe { core::slice::from_raw_parts(self.start, self.len) }
    }

    pub fn release(self) -> Result<usize, RomReleaseError> {
        Syscall::rom_release()
    }

    /// A view over a fabricated `RomRef`; the caller keeps the bytes
    /// alive for the view's whole life.
    /// See `docs/rationale/rom-map-release.md`.
    #[cfg(test)]
    pub(crate) unsafe fn from_reference(reference: RomRef) -> RomView {
        RomView {
            start: reference.start as *const u8,
            len: reference.len as usize,
        }
    }
}

pub fn find<'a>(image: &RomImage<'a>, name: &str) -> Option<&'a [u8]> {
    let entry = image.files().find(|candidate| candidate.name == name)?;
    Some(&image.bytes()[entry.offset..entry.offset + entry.len])
}

#[cfg(test)]
mod tests {
    use super::*;
    use system::rom::encode;

    #[test]
    fn find_reads_the_named_program_bytes_through_a_fabricated_reference() {
        let image = encode(&[("init", b"elf-init".as_slice()), ("shell", b"elf-shell".as_slice())]);
        let reference = RomRef {
            start: image.as_ptr() as u64,
            len: image.len() as u64,
        };
        let view = unsafe { RomView::from_reference(reference) };
        let rom = RomImage::parse(view.image()).unwrap();
        assert_eq!(find(&rom, "init"), Some(b"elf-init".as_slice()));
        assert_eq!(find(&rom, "shell"), Some(b"elf-shell".as_slice()));
        assert_eq!(find(&rom, "nope"), None);
    }
}
