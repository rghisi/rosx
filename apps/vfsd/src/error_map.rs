use block_protocol::client::ClientError;
use block_protocol::wire::BlockError;
use fs_core::error::FsError;
use vfs_protocol::wire::VfsError;

pub(crate) fn fs(error: FsError) -> VfsError {
    match error {
        FsError::NotFound => VfsError::NotFound,
        FsError::AlreadyExists => VfsError::AlreadyExists,
        FsError::InvalidPath | FsError::PathTooLong | FsError::NameTooLong | FsError::InvalidName => {
            VfsError::InvalidPath
        }
        FsError::BadHandle => VfsError::BadHandle,
        FsError::Unsupported => VfsError::Unsupported,
        FsError::NotADirectory => VfsError::NotDirectory,
        FsError::IsADirectory => VfsError::IsADirectory,
        FsError::DirectoryNotEmpty => VfsError::DirectoryNotEmpty,
        _ => VfsError::ServerResource,
    }
}

pub(crate) fn mount(error: FsError) -> VfsError {
    match error {
        FsError::Corrupt | FsError::DeviceTooSmall => VfsError::NotFormatted,
        other => fs(other),
    }
}

pub(crate) fn block(error: ClientError) -> VfsError {
    match error {
        ClientError::Block(BlockError::NotFound) => VfsError::NotFound,
        ClientError::Block(BlockError::Unsupported) => VfsError::Unsupported,
        ClientError::NoConnection => VfsError::NotFound,
        _ => VfsError::ServerResource,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fs_translates_named_errors_and_swallows_the_rest() {
        assert_eq!(fs(FsError::NotFound), VfsError::NotFound);
        assert_eq!(fs(FsError::AlreadyExists), VfsError::AlreadyExists);
        assert_eq!(fs(FsError::InvalidPath), VfsError::InvalidPath);
        assert_eq!(fs(FsError::PathTooLong), VfsError::InvalidPath);
        assert_eq!(fs(FsError::NameTooLong), VfsError::InvalidPath);
        assert_eq!(fs(FsError::InvalidName), VfsError::InvalidPath);
        assert_eq!(fs(FsError::BadHandle), VfsError::BadHandle);
        assert_eq!(fs(FsError::Unsupported), VfsError::Unsupported);
        assert_eq!(fs(FsError::NotADirectory), VfsError::NotDirectory);
        assert_eq!(fs(FsError::IsADirectory), VfsError::IsADirectory);
        assert_eq!(fs(FsError::DirectoryNotEmpty), VfsError::DirectoryNotEmpty);
        assert_eq!(fs(FsError::Corrupt), VfsError::ServerResource);
        assert_eq!(fs(FsError::DeviceTooSmall), VfsError::ServerResource);
    }

    #[test]
    fn mount_answers_not_formatted_where_fs_answers_server_resource() {
        assert_eq!(mount(FsError::Corrupt), VfsError::NotFormatted);
        assert_eq!(mount(FsError::DeviceTooSmall), VfsError::NotFormatted);
        assert_eq!(mount(FsError::NotFound), VfsError::NotFound);
        assert_eq!(mount(FsError::InvalidPath), VfsError::InvalidPath);
    }

    #[test]
    fn block_maps_provider_faults_onto_wire_errors() {
        assert_eq!(block(ClientError::Block(BlockError::NotFound)), VfsError::NotFound);
        assert_eq!(
            block(ClientError::Block(BlockError::Unsupported)),
            VfsError::Unsupported
        );
        assert_eq!(block(ClientError::NoConnection), VfsError::NotFound);
        assert_eq!(block(ClientError::Transport), VfsError::ServerResource);
    }
}
