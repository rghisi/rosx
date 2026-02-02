use core::fmt;
use system::syscall_numbers::SyscallNum;
use system::message::{Message, MessageData, MessageType};
use crate::arch;

pub struct Syscall {}

impl Syscall {
    pub fn exec(entrypoint: usize) {
        arch::raw_syscall(SyscallNum::Exec as u64, entrypoint as u64, 0, 0);
    }

    pub fn task_yield() {
        arch::raw_syscall(SyscallNum::Yield as u64, 0, 0, 0);
    }

    pub fn sleep(ms: u64) {
        arch::raw_syscall(SyscallNum::Sleep as u64, ms, 0, 0);
    }

    pub fn print(args: fmt::Arguments) {
        let message = Message {
            message_type: MessageType::Exec,
            data: MessageData::FmtArgs { args },
        };
        arch::raw_syscall(SyscallNum::Print as u64, &message as *const _ as u64, 0, 0);
    }
}
