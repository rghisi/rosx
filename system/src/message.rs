use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;

pub enum MessageType {
    FileRead,
    FileWrite,
    FileOpen,
    FileClose,
    Exec,
}
pub struct Message<'a> {
    pub message_type: MessageType,
    pub data: MessageData<'a>,
}

pub enum MessageData<'a> {
    Vec { vec: Vec<u8> },
    FmtArgs { args: fmt::Arguments<'a> },
}

pub enum Exec {
    Invalid = 0,
    ThreadSleep = 1,
    Print = 2,
}

impl Exec {
    pub fn from_u8(value: u8) -> Exec {
        match value {
            0 => Exec::Invalid,
            1 => Exec::ThreadSleep,
            2 => Exec::Print,
            _ => Exec::Invalid,
        }
    }
}
