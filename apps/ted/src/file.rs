use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use vfs_protocol::client::{ClientError, Vfs, MODE_CREATE, MODE_WRITE};
use vfs_protocol::wire::VfsError;

pub enum LoadOutcome {
    Exists(Vec<u8>),
    Absent,
}

pub fn root_relative(path: &str) -> String {
    if path.starts_with('/') {
        String::from(path)
    } else {
        format!("/{path}")
    }
}

pub fn load(vfs: &Vfs, path: &str) -> Result<LoadOutcome, String> {
    match vfs.read_whole(path) {
        Ok(bytes) => Ok(LoadOutcome::Exists(bytes)),
        Err(ClientError::Vfs(VfsError::NotFound)) => Ok(LoadOutcome::Absent),
        Err(error) => Err(format!("cannot read {path}: {error:?}")),
    }
}

pub fn save(
    vfs: &Vfs,
    path: &str,
    bytes: &[u8],
    on_disk: Option<usize>,
) -> Result<(), String> {
    let shrinks = on_disk.is_some_and(|size| bytes.len() < size);
    let needs_create = on_disk.is_none() || shrinks;
    if shrinks {
        vfs.delete(path)
            .map_err(|error| format!("cannot retire {path} for the shorter save: {error:?}"))?;
    }
    let mode = if needs_create {
        MODE_WRITE | MODE_CREATE
    } else {
        MODE_WRITE
    };
    let mut file = vfs
        .open(path, mode)
        .map_err(|error| format!("cannot open {path} for writing: {error:?}"))?;
    file.write(bytes)
        .map_err(|error| format!("cannot write {path}: {error:?}"))?;
    file.close()
        .map_err(|error| format!("cannot close {path}: {error:?}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::root_relative;

    #[test]
    fn bare_names_become_root_relative() {
        assert_eq!(root_relative("notes.txt"), "/notes.txt");
    }

    #[test]
    fn absolute_names_are_kept_verbatim() {
        assert_eq!(root_relative("/hello.txt"), "/hello.txt");
        assert_eq!(root_relative("/"), "/");
    }
}
