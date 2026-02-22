#[repr(usize)]
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum SyscallNum {
    Print = 0,
    Sleep = 1,
    Exec = 2,
    Yield = 3,
    ReadChar = 4,
    WaitFuture = 5,
    IsFutureCompleted = 6,
    Alloc = 7,
    Dealloc = 8,
    TryReadChar = 9,
    LoadElf = 10,
    IpcFind = 11,
    IpcSend = 12,
}

impl TryFrom<usize> for SyscallNum {
    type Error = ();

    fn try_from(v: usize) -> Result<Self, ()> {
        match v {
            0 => Ok(Self::Print),
            1 => Ok(Self::Sleep),
            2 => Ok(Self::Exec),
            3 => Ok(Self::Yield),
            4 => Ok(Self::ReadChar),
            5 => Ok(Self::WaitFuture),
            6 => Ok(Self::IsFutureCompleted),
            7 => Ok(Self::Alloc),
            8 => Ok(Self::Dealloc),
            9 => Ok(Self::TryReadChar),
            10 => Ok(Self::LoadElf),
            11 => Ok(Self::IpcFind),
            12 => Ok(Self::IpcSend),
            _ => Err(()),
        }
    }
}
