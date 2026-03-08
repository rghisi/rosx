use alloc::boxed::Box;
use alloc::collections::{BTreeMap, VecDeque};
use alloc::string::String;
use alloc::vec::Vec;
use collections::generational_arena::GenerationalArena;
use system::future::{ Future, FutureHandle };
use system::ipc::IpcError::ServerNotFound;
use system::ipc::{IpcError, IpcReply, IpcReplyFuture, IpcReceiveFuture, IpcReceiveMessage, IpcSendMessage, IpcServerHandle};
use crate::ipc::ipc_server::IpcServerConnection;
use crate::kernel::kernel;
use crate::kernel_services::services;

pub(crate) struct IpcReplyMessage {
    pub value: u32,
    pub future: FutureHandle
}

type Mailbox = VecDeque<IpcReceiveMessage>;

pub(crate) struct IpcManager {
    bindings: GenerationalArena<IpcServerConnection, 256>,
    mailboxes: Vec<Mailbox>,
    registry: BTreeMap<String, IpcServerHandle>,
    receivers: BTreeMap<IpcServerHandle, FutureHandle>,
}

impl IpcManager {

    pub(crate) fn new() -> IpcManager {
        IpcManager {
            bindings: GenerationalArena::new(),
            mailboxes: Vec::new(),
            registry: BTreeMap::new(),
            receivers: BTreeMap::new(),
        }
    }

    pub(crate) fn find(&self, service: &str) -> Result<IpcServerHandle, IpcError> {
        if let Some(handle) = self.registry.get(service) {
            Ok(*handle)
        } else {
            Err(ServerNotFound)
        }
    }

    pub(crate) fn register(&mut self, service: &str) -> Result<IpcServerHandle, IpcError> {
        if self.registry.contains_key(service) {
           return Err(IpcError::ServerCannotBeAdded);
        }

        let binding = IpcServerConnection::new(String::from(service));
        let handle = self.bindings.add(binding).unwrap();
        self.mailboxes.insert(handle.index as usize, Mailbox::new());
        self.registry.insert(String::from(service), handle);

        Ok(handle)
    }

    pub(crate) fn send(&mut self, handle: IpcServerHandle, message: IpcSendMessage) -> FutureHandle {
        let sender = kernel().execution_state.current_task.unwrap();
        let future = Box::new(IpcReplyFuture { reply: None });
        let future_handle = services().future_registry.borrow_mut().register(future).unwrap();
        let receive_message = IpcReceiveMessage {
            value: message.value,
            sender,
            future: future_handle,
        };

        if let Some(receive_future_handle) = self.receivers.remove(&handle) {
            if let Ok(future) = services().future_registry.borrow_mut().arena.borrow_mut(receive_future_handle) {
                let dyn_f: &mut dyn Future = future.as_mut();
                if let Some(rf) = dyn_f.as_any_mut().downcast_mut::<IpcReceiveFuture>() {
                    rf.message = Some(receive_message);
                }
            }
            services().future_registry.borrow_mut().complete(receive_future_handle);
        } else {
            let mailbox = self.mailboxes.get_mut(handle.index as usize).unwrap();
            mailbox.push_back(receive_message);
        }

        future_handle
    }

    pub(crate) fn receive(&mut self, handle: IpcServerHandle) -> Option<IpcReceiveMessage> {
        let mailbox = self.mailboxes.get_mut(handle.index as usize).unwrap();
        if mailbox.is_empty() {
            None
        } else {
            mailbox.pop_front()
        }
    }

    pub(crate) fn wait_receive(&mut self, handle: IpcServerHandle) -> FutureHandle {
        let mailbox = self.mailboxes.get_mut(handle.index as usize).unwrap();
        if let Some(message) = mailbox.pop_front() {
            let future = Box::new(IpcReceiveFuture { message: Some(message) });
            services().future_registry.borrow_mut().register(future).unwrap()
        } else {
            let future = Box::new(IpcReceiveFuture::new());
            let future_handle = services().future_registry.borrow_mut().register(future).unwrap();
            self.receivers.insert(handle, future_handle);
            future_handle
        }
    }

    pub(crate) fn reply(&self, reply: IpcReplyMessage) {
        let future_handle = reply.future;
        let reply_message = IpcReply { value: reply.value };
        if let Ok(future) = services().future_registry.borrow_mut().arena.borrow_mut(future_handle) {
            let dyn_f: &mut dyn Future = future.as_mut();
            if let Some(rf) = dyn_f.as_any_mut().downcast_mut::<IpcReplyFuture>() {
                rf.reply = Some(reply_message);
            }
        }
        services().future_registry.borrow_mut().complete(future_handle);
    }

}