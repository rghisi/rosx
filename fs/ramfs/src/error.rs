#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FsError {
    NotFound,
    AlreadyExists,
    InvalidPath,
    PathTooLong,
    NameTooLong,
    BadHandle,
    TooManyOpen,
}
