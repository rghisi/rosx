use crate::syscall::Syscall;
use alloc::vec::Vec;
use system::rom::RomImage;

const READ_CHUNK_BYTES: usize = 4096;

pub fn read_image() -> Vec<u8> {
    let mut image = Vec::new();
    let mut offset: u64 = 0;
    loop {
        let mut chunk = [0u8; READ_CHUNK_BYTES];
        let copied = Syscall::rom_read(offset, &mut chunk);
        if copied == 0 {
            return image;
        }
        image.extend_from_slice(&chunk[..copied]);
        offset += copied as u64;
    }
}

pub fn find<'a>(image: &RomImage<'a>, name: &str) -> Option<&'a [u8]> {
    let entry = image.files().find(|candidate| candidate.name == name)?;
    Some(&image.bytes()[entry.offset..entry.offset + entry.len])
}
