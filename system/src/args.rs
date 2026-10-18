use alloc::string::{String, ToString};
use alloc::vec::Vec;

pub fn encode(args: &[impl AsRef<str>]) -> Vec<u8> {
    let mut blob = Vec::new();
    for (index, arg) in args.iter().enumerate() {
        if index != 0 {
            blob.push(0);
        }
        blob.extend_from_slice(arg.as_ref().as_bytes());
    }
    blob
}

pub fn decode(bytes: &[u8]) -> Vec<String> {
    if bytes.is_empty() {
        return Vec::new();
    }
    bytes.split(|byte| *byte == 0).map(|entry| String::from_utf8_lossy(entry).to_string()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    #[test]
    fn encode_empty_list_produces_empty_blob() {
        assert_eq!(encode(&[] as &[&str]), Vec::<u8>::new());
    }

    #[test]
    fn decode_empty_blob_produces_empty_list() {
        assert_eq!(decode(&[]), Vec::<String>::new());
    }

    #[test]
    fn encode_single_entry_has_no_separator() {
        assert_eq!(encode(&["program"]), b"program".to_vec());
    }

    #[test]
    fn encode_joins_entries_with_single_nul() {
        assert_eq!(encode(&["prog", "one", "two"]), b"prog\0one\0two".to_vec());
    }

    #[test]
    fn decode_splits_entries() {
        assert_eq!(decode(b"prog\0one\0two"), vec!["prog", "one", "two"]);
    }

    #[test]
    fn roundtrip_multiple_entries() {
        let args = ["pi", "12", "extra"];
        assert_eq!(decode(&encode(&args)), vec!["pi", "12", "extra"]);
    }

    #[test]
    fn entry_with_spaces_survives_roundtrip() {
        let args = ["prog", "one two", "three"];
        assert_eq!(decode(&encode(&args)), vec!["prog", "one two", "three"]);
    }

    #[test]
    fn argv_zero_is_first_entry() {
        let args = ["random_gen_server", "--flag"];
        let decoded = decode(&encode(&args));
        assert_eq!(decoded[0], "random_gen_server");
    }

    #[test]
    fn invalid_utf8_entry_decodes_lossy_without_panic() {
        let decoded = decode(b"prog\0\xFF\xFE\0tail");
        assert_eq!(decoded[0], "prog");
        assert_eq!(decoded[2], "tail");
        assert!(decoded[1].contains('\u{FFFD}'));
    }

    #[test]
    fn smuggled_nul_decodes_as_empty_entry() {
        assert_eq!(decode(b"prog\0\0tail"), vec!["prog", "", "tail"]);
    }
}
