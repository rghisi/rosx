use alloc::vec::Vec;

pub const ROM_MAGIC: &[u8; 8] = b"ROSXROM1";
pub const ROM_MAX_NAME_BYTES: usize = 57;

#[derive(Debug, PartialEq)]
pub enum RomError {
    TooShort,
    BadMagic,
    NameEmpty,
    NameTooLong,
    NameNotUtf8,
    DuplicateName,
    BeyondImage,
}

#[derive(Debug, Copy, Clone, PartialEq)]
pub struct RomEntry<'a> {
    pub name: &'a str,
    pub offset: usize,
    pub len: usize,
}

#[derive(Debug, PartialEq)]
pub struct RomImage<'a> {
    bytes: &'a [u8],
    entries: Vec<RomEntry<'a>>,
}

impl<'a> RomImage<'a> {
    pub fn parse(bytes: &'a [u8]) -> Result<RomImage<'a>, RomError> {
        if bytes.len() < 12 {
            return Err(RomError::TooShort);
        }
        if &bytes[0..8] != ROM_MAGIC {
            return Err(RomError::BadMagic);
        }
        let entry_count = u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize;
        let mut entries: Vec<RomEntry<'a>> = Vec::new();
        let mut cursor = 12usize;
        for _ in 0..entry_count {
            let name_len = *bytes.get(cursor).ok_or(RomError::TooShort)? as usize;
            let record_end = cursor.checked_add(1 + name_len + 8).ok_or(RomError::TooShort)?;
            if record_end > bytes.len() {
                return Err(RomError::TooShort);
            }
            if name_len == 0 {
                return Err(RomError::NameEmpty);
            }
            if name_len > ROM_MAX_NAME_BYTES {
                return Err(RomError::NameTooLong);
            }
            let name_start = cursor + 1;
            let name =
                core::str::from_utf8(&bytes[name_start..name_start + name_len])
                    .map_err(|_| RomError::NameNotUtf8)?;
            if entries.iter().any(|seen| seen.name == name) {
                return Err(RomError::DuplicateName);
            }
            let fields = name_start + name_len;
            let offset =
                u32::from_le_bytes(bytes[fields..fields + 4].try_into().unwrap()) as usize;
            let len =
                u32::from_le_bytes(bytes[fields + 4..fields + 8].try_into().unwrap()) as usize;
            if offset as u64 + len as u64 > bytes.len() as u64 {
                return Err(RomError::BeyondImage);
            }
            entries.push(RomEntry { name, offset, len });
            cursor = record_end;
        }
        Ok(RomImage { bytes, entries })
    }

    pub fn files(&self) -> impl Iterator<Item = RomEntry<'a>> + '_ {
        self.entries.iter().copied()
    }

    pub fn bytes(&self) -> &'a [u8] {
        self.bytes
    }
}

pub fn encode(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut names: Vec<&str> = Vec::new();
    let mut records_len: u64 = 0;
    let mut blobs_len: u64 = 0;
    for (name, data) in files {
        assert!(!name.is_empty(), "rom name must not be empty");
        assert!(
            name.as_bytes().len() <= ROM_MAX_NAME_BYTES,
            "rom name exceeds {ROM_MAX_NAME_BYTES} bytes: {name}"
        );
        assert!(!names.contains(name), "duplicate rom name: {name}");
        names.push(name);
        records_len += 1 + name.as_bytes().len() as u64 + 8;
        blobs_len += data.len() as u64;
    }
    let total = 12 + records_len + blobs_len;
    assert!(total <= u32::MAX as u64, "rom image exceeds u32 address space");
    let count = u32::try_from(files.len()).expect("rom file count exceeds u32");
    let mut image: Vec<u8> = Vec::with_capacity(total as usize);
    image.extend_from_slice(ROM_MAGIC);
    image.extend_from_slice(&count.to_le_bytes());
    let mut offset = 12 + records_len;
    for (name, data) in files {
        image.push(name.as_bytes().len() as u8);
        image.extend_from_slice(name.as_bytes());
        image.extend_from_slice(&(offset as u32).to_le_bytes());
        image.extend_from_slice(&(data.len() as u32).to_le_bytes());
        offset += data.len() as u64;
    }
    for (_, data) in files {
        image.extend_from_slice(data);
    }
    image
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(count: u32) -> Vec<u8> {
        let mut bytes = ROM_MAGIC.to_vec();
        bytes.extend_from_slice(&count.to_le_bytes());
        bytes
    }

    fn record(name: &[u8], offset: u32, len: u32) -> Vec<u8> {
        let mut bytes = vec![name.len() as u8];
        bytes.extend_from_slice(name);
        bytes.extend_from_slice(&offset.to_le_bytes());
        bytes.extend_from_slice(&len.to_le_bytes());
        bytes
    }

    fn two_file_image() -> Vec<u8> {
        encode(&[("alpha", b"one".as_slice()), ("beta", b"two2".as_slice())])
    }

    #[test]
    fn magic_constant_is_pinned() {
        assert_eq!(ROM_MAGIC, &[b'R', b'O', b'S', b'X', b'R', b'O', b'M', b'1']);
        assert_eq!(ROM_MAX_NAME_BYTES, 57);
    }

    #[test]
    fn encode_pins_header_entry_and_blob_offsets() {
        let image = two_file_image();
        assert_eq!(&image[0..8], b"ROSXROM1");
        assert_eq!(&image[8..12], &[2, 0, 0, 0]);
        assert_eq!(image[12], 5);
        assert_eq!(&image[13..18], b"alpha");
        assert_eq!(&image[18..22], &[39, 0, 0, 0]);
        assert_eq!(&image[22..26], &[3, 0, 0, 0]);
        assert_eq!(image[26], 4);
        assert_eq!(&image[27..31], b"beta");
        assert_eq!(&image[31..35], &[42, 0, 0, 0]);
        assert_eq!(&image[35..39], &[4, 0, 0, 0]);
        assert_eq!(&image[39..42], b"one");
        assert_eq!(&image[42..46], b"two2");
        assert_eq!(image.len(), 46);
    }

    #[test]
    fn roundtrip_preserves_names_offsets_lengths_and_blobs() {
        let image = two_file_image();
        let parsed = RomImage::parse(&image).expect("encode output must parse");
        let entries: Vec<RomEntry> = parsed.files().collect();
        assert_eq!(
            entries,
            [
                RomEntry { name: "alpha", offset: 39, len: 3 },
                RomEntry { name: "beta", offset: 42, len: 4 },
            ]
        );
        for (entry, expected) in entries.iter().zip([b"one".as_slice(), b"two2".as_slice()]) {
            assert_eq!(&parsed.bytes()[entry.offset..entry.offset + entry.len], expected);
        }
        assert_eq!(parsed.bytes(), image.as_slice());
    }

    #[test]
    fn encode_preserves_caller_order() {
        let image = encode(&[("zeta", b"z".as_slice()), ("alpha", b"a".as_slice())]);
        let parsed = RomImage::parse(&image).unwrap();
        let names: Vec<&str> = parsed.files().map(|entry| entry.name).collect();
        assert_eq!(names, ["zeta", "alpha"]);
    }

    #[test]
    fn zero_count_image_parses_with_no_entries() {
        let image = encode(&[]);
        assert_eq!(image.len(), 12);
        let parsed = RomImage::parse(&image).expect("empty image must parse");
        assert_eq!(parsed.files().count(), 0);
        assert_eq!(parsed.bytes(), image.as_slice());
    }

    #[test]
    fn zero_count_image_ignores_trailing_bytes() {
        let mut image = header(0);
        image.extend_from_slice(b"trailing junk");
        let parsed = RomImage::parse(&image).expect("trailing bytes must not affect count zero");
        assert_eq!(parsed.files().count(), 0);
    }

    #[test]
    fn rejects_image_shorter_than_header() {
        assert_eq!(RomImage::parse(&[]), Err(RomError::TooShort));
        assert_eq!(RomImage::parse(b"ROSXROM1"), Err(RomError::TooShort));
        let mut short = header(0);
        short.pop();
        assert_eq!(RomImage::parse(&short), Err(RomError::TooShort));
    }

    #[test]
    fn rejects_bad_magic() {
        let mut image = encode(&[("a", b"x".as_slice())]);
        image[7] = b'2';
        assert_eq!(RomImage::parse(&image), Err(RomError::BadMagic));
    }

    #[test]
    fn rejects_truncated_entry_record() {
        let mut image = header(1);
        image.extend_from_slice(b"alph");
        assert_eq!(RomImage::parse(&image), Err(RomError::TooShort));
    }

    #[test]
    fn rejects_missing_record_past_count() {
        let mut image = header(2);
        image.extend_from_slice(&record(b"a", 22, 0));
        assert_eq!(RomImage::parse(&image), Err(RomError::TooShort));
    }

    #[test]
    fn rejects_empty_name() {
        let image = [header(1), record(b"", 21, 0)].concat();
        assert_eq!(RomImage::parse(&image), Err(RomError::NameEmpty));
    }

    #[test]
    fn rejects_name_longer_than_bound() {
        let long = alloc::vec![b'k'; 58];
        let image = [header(1), record(&long, 79, 0)].concat();
        assert_eq!(RomImage::parse(&image), Err(RomError::NameTooLong));
    }

    #[test]
    fn rejects_non_utf8_name() {
        let image = [header(1), record(&[0xFF], 22, 0)].concat();
        assert_eq!(RomImage::parse(&image), Err(RomError::NameNotUtf8));
    }

    #[test]
    fn rejects_duplicate_name() {
        let image = [header(2), record(b"dup", 36, 0), record(b"dup", 36, 0)].concat();
        assert_eq!(RomImage::parse(&image), Err(RomError::DuplicateName));
    }

    #[test]
    fn rejects_entry_range_beyond_image() {
        let image = [header(1), record(b"a", 21, 2)].concat();
        assert_eq!(RomImage::parse(&image), Err(RomError::BeyondImage));
    }

    #[test]
    fn entry_range_is_validated_in_u64_not_usize() {
        let image = [header(1), record(b"a", u32::MAX, u32::MAX)].concat();
        assert_eq!(RomImage::parse(&image), Err(RomError::BeyondImage));
    }

    #[test]
    fn maximum_name_at_bound_parses() {
        let name = alloc::vec![b'k'; ROM_MAX_NAME_BYTES];
        let image = [header(1), record(&name, 78, 0)].concat();
        let parsed = RomImage::parse(&image).expect("57-byte name must parse");
        assert_eq!(parsed.files().count(), 1);
    }

    #[test]
    #[should_panic(expected = "rom name must not be empty")]
    fn encode_rejects_empty_name() {
        encode(&[("", b"x".as_slice())]);
    }

    #[test]
    #[should_panic(expected = "rom name exceeds 57 bytes")]
    fn encode_rejects_name_longer_than_bound() {
        let name: alloc::string::String = (0..58).map(|_| 'k').collect();
        encode(&[(name.as_str(), b"x".as_slice())]);
    }

    #[test]
    #[should_panic(expected = "duplicate rom name")]
    fn encode_rejects_duplicate_name() {
        encode(&[("dup", b"a".as_slice()), ("dup", b"b".as_slice())]);
    }
}
