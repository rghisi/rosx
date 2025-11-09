use alloc::vec;
use system::message::{Exec, Message, MessageType};

pub struct Syscall {}

impl Syscall {
    pub fn exec(entrypoint: usize) {
        kernel::syscall::exec(entrypoint);
    }

    pub fn syscall(message: &Message) -> usize {
        kernel::syscall::syscall(message)
    }

    pub fn task_yield() {}

    pub fn sleep(ms: u64) {
        let n0 = ms as u8;
        let n1 = (ms >> 8) as u8;
        let n2 = (ms >> 16) as u8;
        let n3 = (ms >> 24) as u8;
        let message = Message {
            message_type: MessageType::Exec,
            data: vec![Exec::ThreadSleep as u8, n3, n2, n1, n0],
        };
        Syscall::syscall(&message);
    }
}
