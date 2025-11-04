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