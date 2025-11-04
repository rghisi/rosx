use alloc::vec::Vec;
pub enum MessageType {
    FileRead,
    FileWrite,
    FileOpen,
    FileClose,
    Exec
}
pub struct Message {
    pub message_type: MessageType,
    pub data: Vec<u8>,
}

pub enum Exec {
    Invalid = 0,
    ThreadSleep = 1,
}

impl Exec {
    pub fn from_u8(value: u8) -> Exec {
        match value {
            0 => { Exec::Invalid }
            1 => { Exec::ThreadSleep }
            _ => { Exec::Invalid }
        }
    }
}