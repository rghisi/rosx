use crate::error::FsError;
use crate::spec::{MAX_DEPTH, MAX_NAME_BYTES, MAX_PATH_BYTES};
use alloc::vec::Vec;

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

    pub fn get(&self, index: usize) -> Option<&'a [u8]> {
        self.components.get(index).copied()
    }

    pub fn iter(&self) -> impl Iterator<Item = &'a [u8]> + '_ {
        self.components.iter().copied()
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
                return Err(FsError::InvalidName);
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
    use alloc::vec;
    use alloc::vec::Vec;

    fn parts(path: &[u8]) -> Vec<&[u8]> {
        parse(path).unwrap().iter().collect()
    }

    #[test]
    fn root_path_has_no_components() {
        let parsed = parse(b"/").unwrap();
        assert!(parsed.is_root());
        assert_eq!(parsed.len(), 0);
    }

    #[test]
    fn simple_nested_path_splits_on_slash() {
        assert_eq!(
            parts(b"/dir/sub/file"),
            vec![&b"dir"[..], &b"sub"[..], &b"file"[..]]
        );
    }

    #[test]
    fn trailing_slash_is_tolerated() {
        assert_eq!(parts(b"/dir/sub/"), vec![&b"dir"[..], &b"sub"[..]]);
    }

    #[test]
    fn double_slash_root_is_invalid() {
        assert_eq!(parse(b"//"), Err(FsError::InvalidPath));
    }

    #[test]
    fn dot_components_pass_through_unspecialized() {
        assert_eq!(parts(b"/./a/.."), vec![&b"."[..], &b"a"[..], &b".."[..]]);
    }

    #[test]
    fn rejects_missing_leading_slash_and_empty_path() {
        assert_eq!(parse(b"dir/file"), Err(FsError::InvalidPath));
        assert_eq!(parse(b""), Err(FsError::InvalidPath));
    }

    #[test]
    fn rejects_empty_interior_components() {
        assert_eq!(parse(b"/a//b"), Err(FsError::InvalidPath));
        assert_eq!(parse(b"/a/b//"), Err(FsError::InvalidPath));
    }

    #[test]
    fn name_length_boundaries() {
        let path = build_path(&[&vec![b'n'; MAX_NAME_BYTES]]);
        assert_eq!(parse(&path).unwrap().len(), 1);
        let path = build_path(&[&vec![b'n'; MAX_NAME_BYTES + 1]]);
        assert_eq!(parse(&path), Err(FsError::NameTooLong));
    }

    #[test]
    fn component_with_nul_is_invalid_name() {
        assert_eq!(parse(b"/gu\x00ods"), Err(FsError::InvalidName));
    }

    #[test]
    fn high_bytes_are_accepted_byte_transparently() {
        assert_eq!(
            parts(b"/\xff\xfe/\x01"),
            vec![&[0xFF, 0xFE][..], &[0x01][..]]
        );
    }

    #[test]
    fn path_length_boundary_is_exact() {
        let name = vec![b'c'; 7];
        let names: Vec<&[u8]> = vec![name.as_slice(); 512];
        let exact = build_path(&names);
        assert_eq!(exact.len(), MAX_PATH_BYTES);
        assert_eq!(parse(&exact).unwrap().len(), 512);

        let mut longer = build_path(&names);
        longer.push(b'c');
        assert_eq!(longer.len(), MAX_PATH_BYTES + 1);
        assert_eq!(parse(&longer), Err(FsError::PathTooLong));
    }

    #[test]
    fn depth_boundaries() {
        let names: Vec<Vec<u8>> = (0..MAX_DEPTH).map(|_| b"a".to_vec()).collect();
        let refs: Vec<&[u8]> = names.iter().map(|n| n.as_slice()).collect();
        let path = build_path(&refs);
        assert_eq!(parse(&path).unwrap().len(), MAX_DEPTH);

        let mut deeper: Vec<&[u8]> = refs.clone();
        deeper.push(b"b");
        assert_eq!(parse(&build_path(&deeper)), Err(FsError::DepthExceeded));
    }

    fn build_path(components: &[&[u8]]) -> Vec<u8> {
        let mut path = b"/".to_vec();
        for (index, component) in components.iter().enumerate() {
            if index != 0 {
                path.push(b'/');
            }
            path.extend_from_slice(component);
        }
        path
    }
}
