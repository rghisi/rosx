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
    IpcConnect = 11,
    IpcDisconnect = 12,
    IpcSend = 13,
    IpcReceive = 14,
    IpcBind = 15,
    IpcReceiveFromClient = 16,
    IpcSendToClient = 17,
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
            11 => Ok(Self::IpcConnect),
            12 => Ok(Self::IpcDisconnect),
            13 => Ok(Self::IpcSend),
            14 => Ok(Self::IpcReceive),
            15 => Ok(Self::IpcBind),
            16 => Ok(Self::IpcReceiveFromClient),
            17 => Ok(Self::IpcSendToClient),
            _ => Err(()),
        }
    }
}
