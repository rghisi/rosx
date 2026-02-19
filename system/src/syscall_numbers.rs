#[repr(u64)]
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
}

impl TryFrom<u64> for SyscallNum {
    type Error = ();

    fn try_from(v: u64) -> Result<Self, ()> {
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
            _ => Err(()),
        }
    }
}
