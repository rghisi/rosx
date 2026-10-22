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
    IpcBind = 15,
    IpcSendMessage = 20,
    IpcReceiveMessage = 22,
    IpcAcceptMessage = 23,
    IpcDataWrite = 24,
    IpcDataRead = 25,
    IpcDataRelease = 26,
    RomRead = 27,
    WaitsetCreate = 28,
    WaitsetAttach = 29,
    WaitsetDetach = 30,
    WaitsetWait = 31,
    WaitsetDrop = 32,
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
            15 => Ok(Self::IpcBind),
            20 => Ok(Self::IpcSendMessage),
            22 => Ok(Self::IpcReceiveMessage),
            23 => Ok(Self::IpcAcceptMessage),
            24 => Ok(Self::IpcDataWrite),
            25 => Ok(Self::IpcDataRead),
            26 => Ok(Self::IpcDataRelease),
            27 => Ok(Self::RomRead),
            28 => Ok(Self::WaitsetCreate),
            29 => Ok(Self::WaitsetAttach),
            30 => Ok(Self::WaitsetDetach),
            31 => Ok(Self::WaitsetWait),
            32 => Ok(Self::WaitsetDrop),
            _ => Err(()),
        }
    }
}
