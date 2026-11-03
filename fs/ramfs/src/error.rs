#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FsError {
    NotFound,
    AlreadyExists,
    NotADirectory,
    IsADirectory,
    InvalidPath,
    PathTooLong,
    NameTooLong,
    BadHandle,
    TooManyOpen,
}
