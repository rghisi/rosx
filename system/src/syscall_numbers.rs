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
    IpcEndpointCreate = 9,
    IpcSend = 10,
    IpcRecv = 11,
    IpcReply = 12,
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
            9 => Ok(Self::IpcEndpointCreate),
            10 => Ok(Self::IpcSend),
            11 => Ok(Self::IpcRecv),
            12 => Ok(Self::IpcReply),
            _ => Err(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ipc_endpoint_create_round_trips() {
        assert_eq!(SyscallNum::try_from(9), Ok(SyscallNum::IpcEndpointCreate));
        assert_eq!(SyscallNum::IpcEndpointCreate as u64, 9);
    }

    #[test]
    fn ipc_send_round_trips() {
        assert_eq!(SyscallNum::try_from(10), Ok(SyscallNum::IpcSend));
    }

    #[test]
    fn ipc_recv_round_trips() {
        assert_eq!(SyscallNum::try_from(11), Ok(SyscallNum::IpcRecv));
    }

    #[test]
    fn ipc_reply_round_trips() {
        assert_eq!(SyscallNum::try_from(12), Ok(SyscallNum::IpcReply));
    }
}
