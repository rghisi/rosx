use alloc::vec::Vec;

pub const MAX_IMAGE_BYTES: u64 = 8 * 1024 * 1024;

const CHUNK_BYTES: usize = 4096;

pub fn is_loadable_size(size: u64) -> bool {
    size <= MAX_IMAGE_BYTES
}

pub fn is_elf(bytes: &[u8]) -> bool {
    bytes.starts_with(b"\x7fELF")
}

pub fn read_stream(
    size: u64,
    read: &mut dyn FnMut(&mut [u8]) -> Option<usize>,
) -> Option<Vec<u8>> {
    let mut image = Vec::new();
    let mut chunk = [0u8; CHUNK_BYTES];
    while (image.len() as u64) < size {
        let wanted = (size - image.len() as u64).min(CHUNK_BYTES as u64) as usize;
        match read(&mut chunk[..wanted]) {
            Some(0) | None => return None,
            Some(count) => image.extend_from_slice(&chunk[..count]),
        }
    }
    Some(image)
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    fn script(chunks: Vec<Vec<u8>>) -> impl FnMut(&mut [u8]) -> Option<usize> {
        let mut chunks = chunks.into_iter();
        move |buf| match chunks.next() {
            Some(bytes) => {
                let count = bytes.len().min(buf.len());
                buf[..count].copy_from_slice(&bytes[..count]);
                Some(count)
            }
            None => Some(0),
        }
    }

    #[test]
    fn reads_until_declared_size() {
        let mut read = script(vec![vec![1, 2, 3], vec![4, 5]]);
        assert_eq!(read_stream(5, &mut read), Some(vec![1, 2, 3, 4, 5]));
    }

    #[test]
    fn short_reads_are_tolerated() {
        let mut read = script(vec![vec![7], vec![7], vec![7]]);
        assert_eq!(read_stream(3, &mut read), Some(vec![7, 7, 7]));
    }

    #[test]
    fn early_end_of_stream_is_rejected() {
        let mut read = script(vec![vec![1]]);
        assert_eq!(read_stream(4, &mut read), None);
    }

    #[test]
    fn read_errors_are_rejected() {
        let mut read = |_buf: &mut [u8]| None;
        assert_eq!(read_stream(8, &mut read), None);
    }

    #[test]
    fn empty_size_yields_empty_image() {
        let mut read = script(vec![]);
        assert_eq!(read_stream(0, &mut read), Some(Vec::new()));
    }

    #[test]
    fn reads_across_chunk_boundaries() {
        let mut read = |buf: &mut [u8]| {
            buf.fill(0xAA);
            Some(buf.len())
        };
        let image = read_stream(4097, &mut read).expect("stream");
        assert_eq!(image.len(), 4097);
        assert!(image.iter().all(|byte| *byte == 0xAA));
    }

    #[test]
    fn loadable_size_accepts_up_to_the_cap() {
        assert!(is_loadable_size(0));
        assert!(is_loadable_size(MAX_IMAGE_BYTES));
        assert!(!is_loadable_size(MAX_IMAGE_BYTES + 1));
    }

    #[test]
    fn elf_gate_rejects_foreign_bytes() {
        assert!(is_elf(b"\x7fELF\x02\x01"));
        assert!(!is_elf(b"MZ\x90\x00"));
        assert!(!is_elf(b"\x7fEL"));
        assert!(!is_elf(b""));
    }
}
