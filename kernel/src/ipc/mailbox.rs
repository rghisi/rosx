use alloc::collections::VecDeque;
use system::ipc::IpcMessage;

pub(crate) struct Mailbox {
    queue: VecDeque<IpcMessage>
}

impl Mailbox {

    pub fn new() -> Mailbox {
        Mailbox {
            queue: VecDeque::with_capacity(10)
        }
    }

    pub fn push_back(&mut self, message: IpcMessage) {
        self.queue.push_back(message);
    }

    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }

    pub fn pop_front(&mut self) -> Option<IpcMessage> {
        self.queue.pop_front()
    }
}
