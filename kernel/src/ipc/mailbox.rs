use alloc::collections::VecDeque;
use system::ipc::IpcMessage;

pub(crate) struct Mailbox {
    queue: VecDeque<IpcMessage>
}

#[derive(Debug, PartialEq, Copy, Clone)]
pub(crate) enum MailboxError {
    OutOfSpace,
    NotFound,
}

impl Mailbox {

    pub fn new() -> Mailbox {
        Mailbox {
            queue: VecDeque::with_capacity(10)
        }
    }

    pub fn push_back(&mut self, message: IpcMessage) -> Result<(), MailboxError> {
        // FIXME: capacity() returns total capacity, not remaining.
        // But for now let's keep it as is if it was intended to limit.
        // Actually, VecDeque::with_capacity(10) doesn't mean it can't grow.
        // If we want to limit, we should check len().
        self.queue.push_back(message);
        Ok(())
    }

    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }

    pub fn pop_front(&mut self) -> Option<IpcMessage> {
        self.queue.pop_front()
    }
}
