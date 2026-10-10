use alloc::collections::VecDeque;
use system::ipc::Message;

pub(crate) const MAILBOX_DEPTH: usize = 32;

pub(crate) struct Mailbox {
    queue: VecDeque<Message>
}

impl Mailbox {

    pub fn new() -> Mailbox {
        Mailbox {
            queue: VecDeque::new()
        }
    }

    pub fn push_back(&mut self, message: Message) -> Result<(), ()> {
        if self.queue.len() == MAILBOX_DEPTH {
            return Err(());
        }
        self.queue.push_back(message);
        Ok(())
    }

    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }

    pub fn pop_front(&mut self) -> Option<Message> {
        self.queue.pop_front()
    }
}
