use alloc::collections::VecDeque;
use collections::generational_arena::Handle;
use system::ipc::{IpcConnectionError, IpcMessage};
use crate::ipc::mailbox::MailboxError::OutOfSpace;

pub(crate) struct Mailbox {
    queue: VecDeque<IpcMessage>
}

pub(crate) enum MailboxError {
    OutOfSpace,
}

impl Mailbox {

    pub fn new() -> Mailbox {
        Mailbox {
            queue: VecDeque::with_capacity(10)
        }
    }

    pub fn push_back(&mut self, message: IpcMessage) -> Result<(), MailboxError> {
        if self.queue.capacity() == 0 {
            return Err(OutOfSpace);
        }
       self.queue.push_back(message);
        Ok(())
    }

    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }

    pub fn pop_front(&mut self) -> Option<IpcMessage> {
        self.queue.pop_front()
    }

    pub fn pop_front_async(&mut self) -> Option<IpcMessage> {
        self.queue.pop_front()
    }

}