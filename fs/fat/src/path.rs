use fs_core::error::FsError;
use alloc::vec::Vec;

pub const MAX_NAME_BYTES: usize = 255;
pub const MAX_PATH_BYTES: usize = 4096;
pub const MAX_DEPTH: usize = 512;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathParts<'a> {
    components: Vec<&'a [u8]>,
}

impl<'a> PathParts<'a> {
    pub fn is_root(&self) -> bool {
        self.components.is_empty()
    }

    pub fn len(&self) -> usize {
        self.components.len()
    }

    pub fn is_empty(&self) -> bool {
        self.components.is_empty()
    }

    pub fn get(&self, index: usize) -> Option<&'a [u8]> {
        self.components.get(index).copied()
    }

    pub fn last_segment(&self) -> Option<&'a [u8]> {
        self.components.last().copied()
    }
}

pub fn parse(path: &[u8]) -> Result<PathParts<'_>, FsError> {
    if path.len() > MAX_PATH_BYTES {
        return Err(FsError::PathTooLong);
    }
    if path.is_empty() || path[0] != b'/' {
        return Err(FsError::InvalidPath);
    }
    let body = &path[1..];
    let mut components: Vec<&[u8]> = Vec::new();
    if !body.is_empty() {
        let body = if body[body.len() - 1] == b'/' {
            &body[..body.len() - 1]
        } else {
            body
        };
        for raw in body.split(|byte| *byte == b'/') {
            if raw.is_empty() {
                return Err(FsError::InvalidPath);
            }
            if raw.len() > MAX_NAME_BYTES {
                return Err(FsError::NameTooLong);
            }
            if raw.contains(&0) {
                return Err(FsError::InvalidPath);
            }
            components.push(raw);
        }
    }
    if components.len() > MAX_DEPTH {
        return Err(FsError::DepthExceeded);
    }
    Ok(PathParts { components })
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::format;
    use alloc::string::String;
    use alloc::vec;

    fn components_of(path: &[u8]) -> Vec<&[u8]> {
        parse(path).expect("parses").components
    }

    fn name_of_len(length: usize) -> String {
        (0..length).map(|_| 'k').collect()
    }

    #[test]
    fn root_is_the_empty_component_list() {
        assert!(parse(b"/").expect("parses").is_root());
    }

    #[test]
    fn nested_paths_split_into_components() {
        assert_eq!(components_of(b"/a"), vec![b"a".as_slice()]);
        assert_eq!(components_of(b"/a/b/c"), vec![b"a".as_slice(), b"b", b"c"]);
    }

    #[test]
    fn trailing_slash_is_the_same_path() {
        assert_eq!(parse(b"/a/b/").expect("parses"), parse(b"/a/b").expect("parses"));
    }

    #[test]
    fn relative_empty_and_hollow_paths_are_invalid() {
        assert_eq!(parse(b"a"), Err(FsError::InvalidPath));
        assert_eq!(parse(b""), Err(FsError::InvalidPath));
        assert_eq!(parse(b"//a"), Err(FsError::InvalidPath));
        assert_eq!(parse(b"/a//b"), Err(FsError::InvalidPath));
        assert_eq!(parse(b"/a//"), Err(FsError::InvalidPath));
        assert_eq!(parse(b"//"), Err(FsError::InvalidPath));
    }

    #[test]
    fn component_at_limit_is_accepted_and_past_it_is_name_too_long() {
        let at_limit = format!("/{}", name_of_len(255));
        assert_eq!(parse(at_limit.as_bytes()).expect("parses").len(), 1);
        let past_limit = format!("/{}", name_of_len(256));
        assert_eq!(parse(past_limit.as_bytes()), Err(FsError::NameTooLong));
    }

    #[test]
    fn nul_in_any_component_is_invalid_path() {
        assert_eq!(parse(b"/bad\0name"), Err(FsError::InvalidPath));
        assert_eq!(parse(b"/a/b\0c"), Err(FsError::InvalidPath));
    }

    #[test]
    fn path_past_limit_is_path_too_long() {
        let long = format!("/{}", name_of_len(4096));
        assert_eq!(parse(long.as_bytes()), Err(FsError::PathTooLong));
    }

    #[test]
    fn depth_past_limit_is_depth_exceeded() {
        let at_limit: String = (0..MAX_DEPTH).map(|_| "/k").collect();
        assert_eq!(parse(at_limit.as_bytes()).expect("parses").len(), MAX_DEPTH);
        let past_limit: String = (0..MAX_DEPTH + 1).map(|_| "/k").collect();
        assert_eq!(parse(past_limit.as_bytes()), Err(FsError::DepthExceeded));
    }
}
