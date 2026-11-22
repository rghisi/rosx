use crate::once::Once;
use system::rom::RomError;
use system::rom::RomImage;

static ROM_IMAGE: Once<&'static [u8]> = Once::new();

pub fn init(image: &'static [u8]) {
    ROM_IMAGE.call_once(|| image);
}

pub fn image() -> &'static [u8] {
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

pub fn read(offset: u64, dst: &mut [u8]) -> usize {
    read_at(image(), offset, dst)
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
}
