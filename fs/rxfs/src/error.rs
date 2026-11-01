#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IoError {
    Device,
    Flush,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FsError {
    Io(IoError),
    DeviceTooSmall,
    VolumeTooLarge,
}
