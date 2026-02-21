use alloc::vec::Vec;
use system::ipc::Message;
use crate::future::Future;
use crate::kernel_services::services;
use crate::task::TaskHandle;

pub(crate) struct IpcClientFuture(pub(crate) TaskHandle);
pub(crate) struct IpcServerFuture(pub(crate) TaskHandle);

impl Future for IpcClientFuture {
    fn is_completed(&self) -> bool {
        services().endpoint_registry.borrow().has_client_reply(self.0)
    }
}

impl Future for IpcServerFuture {
    fn is_completed(&self) -> bool {
        services().endpoint_registry.borrow().has_server_delivery(self.0)
    }
}

pub(crate) struct EndpointRegistry {
    endpoints: Vec<Endpoint>,
    server_deliveries: Vec<(TaskHandle, ReplyToken, Message)>,
    client_replies: Vec<(TaskHandle, Message)>,
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
pub(crate) struct ReplyToken(pub(crate) TaskHandle);

#[derive(Debug, PartialEq)]
pub(crate) enum RecvOutcome {
    ServerHasMessage(ReplyToken, Message),
    ServerBlocked,
}

#[derive(Debug, PartialEq)]
pub(crate) enum IpcError {
    EndpointNotFound,
    EndpointAlreadyExists,
    EndpointBusy,
}

impl EndpointRegistry {
    pub(crate) fn new() -> Self {
        Self {
            endpoints: Vec::new(),
            server_deliveries: Vec::new(),
            client_replies: Vec::new(),
        }
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

    pub(crate) fn send(&mut self, id: u32, sender: TaskHandle, msg: Message) -> Result<(), IpcError> {
        let endpoint = self.find_mut(id).ok_or(IpcError::EndpointNotFound)?;
        let notify_server = match endpoint.state {
            EndpointState::Idle => {
                endpoint.state = EndpointState::WaitingCaller(sender, msg);
                None
            }
            EndpointState::WaitingReceiver(server_task) => {
                endpoint.state = EndpointState::WaitingReply(sender);
                Some(server_task)
            }
            EndpointState::WaitingCaller(_, _) | EndpointState::WaitingReply(_) => {
                return Err(IpcError::EndpointBusy);
            }
        };
        if let Some(server_task) = notify_server {
            self.server_deliveries.push((server_task, ReplyToken(sender), msg));
        }
        Ok(())
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
                Ok(RecvOutcome::ServerHasMessage(ReplyToken(client_task), msg))
            }
            EndpointState::WaitingReceiver(_) | EndpointState::WaitingReply(_) => {
                Err(IpcError::EndpointBusy)
            }
        }
    }

    pub(crate) fn reply(&mut self, token: ReplyToken, msg: Message) {
        let ReplyToken(client_task) = token;
        for endpoint in &mut self.endpoints {
            if let EndpointState::WaitingReply(task) = endpoint.state {
                if task == client_task {
                    endpoint.state = EndpointState::Idle;
                    break;
                }
            }
        }
        self.client_replies.push((client_task, msg));
    }

    pub(crate) fn has_server_delivery(&self, server: TaskHandle) -> bool {
        self.server_deliveries.iter().any(|(t, _, _)| *t == server)
    }

    pub(crate) fn take_server_delivery(&mut self, server: TaskHandle) -> Option<(ReplyToken, Message)> {
        let pos = self.server_deliveries.iter().position(|(t, _, _)| *t == server)?;
        let (_, token, msg) = self.server_deliveries.swap_remove(pos);
        Some((token, msg))
    }

    pub(crate) fn has_client_reply(&self, client: TaskHandle) -> bool {
        self.client_replies.iter().any(|(t, _)| *t == client)
    }

    pub(crate) fn take_client_reply(&mut self, client: TaskHandle) -> Option<Message> {
        let pos = self.client_replies.iter().position(|(t, _)| *t == client)?;
        let (_, msg) = self.client_replies.swap_remove(pos);
        Some(msg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use collections::generational_arena::Handle;

    fn task(id: u8) -> TaskHandle {
        Handle::new(id, 0u8)
    }

    fn msg(tag: u64) -> Message {
        Message::new(tag)
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
        assert_eq!(reg.send(99, task(0), msg(0)), Err(IpcError::EndpointNotFound));
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

        assert!(reg.send(1, task(0), msg(0)).is_ok());
    }

    #[test]
    fn send_with_waiting_server_puts_message_in_server_inbox() {
        let mut reg = EndpointRegistry::new();
        reg.create(1).unwrap();
        reg.recv(1, task(10)).unwrap();

        reg.send(1, task(20), msg(42)).unwrap();

        assert!(reg.has_server_delivery(task(10)));
        let (_, delivered_msg) = reg.take_server_delivery(task(10)).unwrap();
        assert_eq!(delivered_msg.tag, 42);
    }

    #[test]
    fn recv_with_waiting_client_delivers_message_directly() {
        let mut reg = EndpointRegistry::new();
        reg.create(1).unwrap();
        reg.send(1, task(20), msg(42)).unwrap();

        let outcome = reg.recv(1, task(10)).unwrap();
        assert!(matches!(outcome, RecvOutcome::ServerHasMessage(_, m) if m.tag == 42));
    }

    #[test]
    fn reply_puts_message_in_client_inbox() {
        let mut reg = EndpointRegistry::new();
        reg.create(1).unwrap();
        reg.send(1, task(20), msg(0)).unwrap();
        let RecvOutcome::ServerHasMessage(token, _) = reg.recv(1, task(10)).unwrap() else { panic!() };

        reg.reply(token, msg(7));

        assert!(reg.has_client_reply(task(20)));
        let reply = reg.take_client_reply(task(20)).unwrap();
        assert_eq!(reply.tag, 7);
    }

    #[test]
    fn endpoint_returns_to_idle_after_full_exchange() {
        let mut reg = EndpointRegistry::new();
        reg.create(1).unwrap();
        reg.send(1, task(20), msg(0)).unwrap();
        let RecvOutcome::ServerHasMessage(token, _) = reg.recv(1, task(10)).unwrap() else { panic!() };
        reg.reply(token, msg(0));

        let outcome = reg.recv(1, task(10)).unwrap();
        assert!(matches!(outcome, RecvOutcome::ServerBlocked));
    }

    #[test]
    fn second_send_while_client_waiting_returns_busy() {
        let mut reg = EndpointRegistry::new();
        reg.create(1).unwrap();
        reg.send(1, task(20), msg(0)).unwrap();

        assert_eq!(reg.send(1, task(21), msg(0)), Err(IpcError::EndpointBusy));
    }

    #[test]
    fn take_server_delivery_removes_it_from_inbox() {
        let mut reg = EndpointRegistry::new();
        reg.create(1).unwrap();
        reg.recv(1, task(10)).unwrap();
        reg.send(1, task(20), msg(0)).unwrap();

        reg.take_server_delivery(task(10));

        assert!(!reg.has_server_delivery(task(10)));
    }

    #[test]
    fn take_client_reply_removes_it_from_inbox() {
        let mut reg = EndpointRegistry::new();
        reg.create(1).unwrap();
        reg.send(1, task(20), msg(0)).unwrap();
        let RecvOutcome::ServerHasMessage(token, _) = reg.recv(1, task(10)).unwrap() else { panic!() };
        reg.reply(token, msg(0));

        reg.take_client_reply(task(20));

        assert!(!reg.has_client_reply(task(20)));
    }

    #[test]
    fn server_inbox_empty_when_no_delivery() {
        let mut reg = EndpointRegistry::new();
        assert!(!reg.has_server_delivery(task(10)));
    }

    #[test]
    fn client_inbox_empty_when_no_reply() {
        let mut reg = EndpointRegistry::new();
        assert!(!reg.has_client_reply(task(20)));
    }
}
