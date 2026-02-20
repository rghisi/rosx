use alloc::vec::Vec;
use system::ipc::Message;
use crate::task::TaskHandle;

pub(crate) struct EndpointRegistry {
    endpoints: Vec<Endpoint>,
}

struct Endpoint {
    id: u32,
    state: EndpointState,
}

#[derive(Copy, Clone)]
enum EndpointState {
    Idle,
    WaitingReceiver(TaskHandle),
    WaitingCaller(TaskHandle, Message),
    WaitingReply(TaskHandle),
}

#[derive(Debug, PartialEq)]
pub(crate) struct ReplyToken(TaskHandle);

#[derive(Debug, PartialEq)]
pub(crate) enum SendOutcome {
    ServerWasWaiting(TaskHandle),
    Queued,
}

#[derive(Debug, PartialEq)]
pub(crate) enum RecvOutcome {
    ClientWasWaiting(ReplyToken, Message),
    ServerBlocked,
}

pub(crate) struct ReplyOutcome {
    pub(crate) client_task: TaskHandle,
    pub(crate) message: Message,
}

#[derive(Debug, PartialEq)]
pub(crate) enum IpcError {
    EndpointNotFound,
    EndpointAlreadyExists,
    EndpointBusy,
}

impl EndpointRegistry {
    pub(crate) fn new() -> Self {
        Self { endpoints: Vec::new() }
    }

    fn find_mut(&mut self, id: u32) -> Option<&mut Endpoint> {
        self.endpoints.iter_mut().find(|e| e.id == id)
    }

    pub(crate) fn create(&mut self, id: u32) -> Result<(), IpcError> {
        if self.endpoints.iter().any(|e| e.id == id) {
            return Err(IpcError::EndpointAlreadyExists);
        }
        self.endpoints.push(Endpoint { id, state: EndpointState::Idle });
        Ok(())
    }

    pub(crate) fn send(&mut self, id: u32, sender: TaskHandle, msg: Message) -> Result<SendOutcome, IpcError> {
        let endpoint = self.find_mut(id).ok_or(IpcError::EndpointNotFound)?;
        match endpoint.state {
            EndpointState::Idle => {
                endpoint.state = EndpointState::WaitingCaller(sender, msg);
                Ok(SendOutcome::Queued)
            }
            EndpointState::WaitingReceiver(server_task) => {
                endpoint.state = EndpointState::WaitingReply(sender);
                Ok(SendOutcome::ServerWasWaiting(server_task))
            }
            EndpointState::WaitingCaller(_, _) | EndpointState::WaitingReply(_) => {
                Err(IpcError::EndpointBusy)
            }
        }
    }

    pub(crate) fn recv(&mut self, id: u32, server: TaskHandle) -> Result<RecvOutcome, IpcError> {
        let endpoint = self.find_mut(id).ok_or(IpcError::EndpointNotFound)?;
        match endpoint.state {
            EndpointState::Idle => {
                endpoint.state = EndpointState::WaitingReceiver(server);
                Ok(RecvOutcome::ServerBlocked)
            }
            EndpointState::WaitingCaller(client_task, msg) => {
                endpoint.state = EndpointState::WaitingReply(client_task);
                Ok(RecvOutcome::ClientWasWaiting(ReplyToken(client_task), msg))
            }
            EndpointState::WaitingReceiver(_) | EndpointState::WaitingReply(_) => {
                Err(IpcError::EndpointBusy)
            }
        }
    }

    pub(crate) fn reply(&mut self, token: ReplyToken, msg: Message) -> ReplyOutcome {
        let ReplyToken(client_task) = token;
        for endpoint in &mut self.endpoints {
            if let EndpointState::WaitingReply(task) = endpoint.state {
                if task == client_task {
                    endpoint.state = EndpointState::Idle;
                    return ReplyOutcome { client_task, message: msg };
                }
            }
        }
        ReplyOutcome { client_task, message: msg }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use collections::generational_arena::Handle;

    fn task(id: u8) -> TaskHandle {
        Handle::new(id, 0u8)
    }

    fn empty_msg() -> Message {
        Message::new(0)
    }

    #[test]
    fn create_registers_endpoint() {
        let mut reg = EndpointRegistry::new();
        assert!(reg.create(1).is_ok());
    }

    #[test]
    fn create_duplicate_returns_error() {
        let mut reg = EndpointRegistry::new();
        reg.create(1).unwrap();
        assert_eq!(reg.create(1), Err(IpcError::EndpointAlreadyExists));
    }

    #[test]
    fn recv_on_unknown_endpoint_returns_error() {
        let mut reg = EndpointRegistry::new();
        assert_eq!(reg.recv(99, task(0)), Err(IpcError::EndpointNotFound));
    }

    #[test]
    fn send_on_unknown_endpoint_returns_error() {
        let mut reg = EndpointRegistry::new();
        assert_eq!(reg.send(99, task(0), empty_msg()), Err(IpcError::EndpointNotFound));
    }

    #[test]
    fn recv_on_idle_endpoint_blocks_server() {
        let mut reg = EndpointRegistry::new();
        reg.create(1).unwrap();

        let outcome = reg.recv(1, task(0)).unwrap();
        assert!(matches!(outcome, RecvOutcome::ServerBlocked));
    }

    #[test]
    fn send_with_no_server_queues_client() {
        let mut reg = EndpointRegistry::new();
        reg.create(1).unwrap();

        let outcome = reg.send(1, task(0), empty_msg()).unwrap();
        assert!(matches!(outcome, SendOutcome::Queued));
    }

    #[test]
    fn send_wakes_waiting_server() {
        let mut reg = EndpointRegistry::new();
        reg.create(1).unwrap();
        reg.recv(1, task(10)).unwrap();

        let outcome = reg.send(1, task(20), empty_msg()).unwrap();
        assert!(matches!(outcome, SendOutcome::ServerWasWaiting(h) if h == task(10)));
    }

    #[test]
    fn recv_delivers_queued_message_to_server() {
        let mut reg = EndpointRegistry::new();
        reg.create(1).unwrap();
        let msg = Message::new(42).with_word(0, 99);
        reg.send(1, task(20), msg).unwrap();

        let outcome = reg.recv(1, task(10)).unwrap();
        assert!(matches!(outcome, RecvOutcome::ClientWasWaiting(_, m) if m.tag == 42 && m.words[0] == 99));
    }

    #[test]
    fn reply_returns_client_task_and_message() {
        let mut reg = EndpointRegistry::new();
        reg.create(1).unwrap();
        reg.send(1, task(20), empty_msg()).unwrap();
        let outcome = reg.recv(1, task(10)).unwrap();
        let RecvOutcome::ClientWasWaiting(token, _) = outcome else { panic!("wrong outcome") };

        let reply_msg = Message::new(7).with_word(0, 123);
        let result = reg.reply(token, reply_msg);
        assert_eq!(result.client_task, task(20));
        assert_eq!(result.message.tag, 7);
        assert_eq!(result.message.words[0], 123);
    }

    #[test]
    fn endpoint_returns_to_idle_after_full_exchange() {
        let mut reg = EndpointRegistry::new();
        reg.create(1).unwrap();
        reg.send(1, task(20), empty_msg()).unwrap();
        let outcome = reg.recv(1, task(10)).unwrap();
        let RecvOutcome::ClientWasWaiting(token, _) = outcome else { panic!("wrong outcome") };
        reg.reply(token, empty_msg());

        let outcome = reg.recv(1, task(10)).unwrap();
        assert!(matches!(outcome, RecvOutcome::ServerBlocked));
    }

    #[test]
    fn second_send_while_client_waiting_returns_busy() {
        let mut reg = EndpointRegistry::new();
        reg.create(1).unwrap();
        reg.send(1, task(20), empty_msg()).unwrap();

        assert_eq!(reg.send(1, task(21), empty_msg()), Err(IpcError::EndpointBusy));
    }
}
