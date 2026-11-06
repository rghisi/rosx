use crate::once::Once;

static ROM_IMAGE: Once<&'static [u8]> = Once::new();

pub fn init(image: &'static [u8]) {
    ROM_IMAGE.call_once(|| image);
}

pub fn image() -> &'static [u8] {
    ROM_IMAGE.get().copied().unwrap_or(&[])
}

pub fn read(offset: u64, dst: &mut [u8]) -> usize {
    read_at(image(), offset, dst)
}

pub fn file_in<'a>(image: &'a [u8], name: &str) -> Option<&'a [u8]> {
    let rom = system::rom::RomImage::parse(image).ok()?;
    let record = rom.files().find(|record| record.name == name)?;
    rom.bytes().get(record.offset..record.offset + record.len)
}

pub fn read_at(image: &[u8], offset: u64, dst: &mut [u8]) -> usize {
    if offset >= image.len() as u64 {
        return 0;
    }
    let start = offset as usize;
    let count = core::cmp::min(dst.len(), image.len() - start);
    dst[..count].copy_from_slice(&image[start..start + count]);
    count
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_at_empty_image_copies_nothing() {
        let mut dst = [0u8; 4];
        assert_eq!(read_at(&[], 0, &mut dst), 0);
    }

    #[test]
    fn read_at_start_copies_exact_prefix() {
        let mut dst = [0u8; 3];
        assert_eq!(read_at(b"abcdef", 0, &mut dst), 3);
        assert_eq!(&dst, b"abc");
    }

    #[test]
    fn read_at_middle_copies_exact_window() {
        let mut dst = [0u8; 2];
        assert_eq!(read_at(b"abcdef", 2, &mut dst), 2);
        assert_eq!(&dst, b"cd");
    }

    #[test]
    fn read_at_clamps_across_end() {
        let mut dst = [0u8; 8];
        assert_eq!(read_at(b"abcdef", 4, &mut dst), 2);
        assert_eq!(&dst[..2], b"ef");
    }

    #[test]
    fn read_at_offset_at_len_is_zero() {
        let mut dst = [0u8; 4];
        assert_eq!(read_at(b"abcdef", 6, &mut dst), 0);
    }

    #[test]
    fn read_at_offset_past_end_is_zero() {
        let mut dst = [0u8; 4];
        assert_eq!(read_at(b"abcdef", 99, &mut dst), 0);
    }

    #[test]
    fn read_at_u64_max_offset_is_zero() {
        let mut dst = [0u8; 4];
        assert_eq!(read_at(b"abcdef", u64::MAX, &mut dst), 0);
    }

    #[test]
    fn read_at_zero_length_dst_copies_nothing() {
        assert_eq!(read_at(b"abcdef", 0, &mut []), 0);
    }

    #[test]
    fn init_registers_image_for_read() {
        static IMAGE: &[u8] = b"kernel rom image";
        init(IMAGE);
        assert_eq!(image(), IMAGE);
        let mut dst = [0u8; 6];
        assert_eq!(read(0, &mut dst), 6);
        assert_eq!(&dst, b"kernel");
        let mut tail = [0u8; 64];
        assert_eq!(read(11, &mut tail), 5);
        assert_eq!(&tail[..5], b"image");
        assert_eq!(read(16, &mut tail), 0);
    }

    #[test]
    fn file_in_returns_the_recorded_bytes() {
        let image = system::rom::encode(&[
            ("vfsd", b"elf-vfsd".as_slice()),
            ("shell", b"elf-shell".as_slice()),
        ]);
        assert_eq!(file_in(&image, "vfsd"), Some(b"elf-vfsd".as_slice()));
        assert_eq!(file_in(&image, "shell"), Some(b"elf-shell".as_slice()));
    }

    #[test]
    fn file_in_answers_none_for_unknown_names() {
        let image = system::rom::encode(&[("vfsd", b"elf".as_slice())]);
        assert_eq!(file_in(&image, "shell"), None);
    }

    #[test]
    fn file_in_answers_none_for_malformed_images() {
        assert_eq!(file_in(b"", "vfsd"), None);
        assert_eq!(file_in(b"not a rom image at all", "vfsd"), None);
    }

    #[test]
    fn file_in_exposes_zero_length_records_as_empty_slices() {
        let image = system::rom::encode(&[("notes", b"".as_slice())]);
        assert_eq!(file_in(&image, "notes"), Some(b"".as_slice()));
    }
}
