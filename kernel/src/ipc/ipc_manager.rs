use alloc::collections::{BTreeMap};
use alloc::string::String;
use collections::generational_arena::{GenerationalArena, Handle};
use system::ipc::{IpcConnectionError, IpcMessage, IpcSendError, IpcConnectionHandle, IpcBindingError, IpcReceiveError, IpcMessageFuture};
use crate::ipc::mailbox_manager::{MailboxManager, MailboxHandle};

struct IpcServerBinding {
    pub service: String,
    pub mailbox_handle: MailboxHandle
}

impl IpcServerBinding {

    pub(crate) fn new(service: String, mailbox_handle: MailboxHandle) -> IpcServerBinding {
        IpcServerBinding {
            service,
            mailbox_handle
        }
    }
}

type IpcBindingHandle = Handle;

struct IpcConnection {
    server_mailbox: MailboxHandle,
    client_mailbox: MailboxHandle,
}

pub(crate) struct IpcManager {
    bindings: GenerationalArena<IpcServerBinding, 256>,
    mailbox_manager: MailboxManager,
    connections: GenerationalArena<IpcConnection, 256>,
    registry: BTreeMap<String, IpcBindingHandle>,
}

impl IpcManager {

    pub(crate) fn new() -> IpcManager {
        IpcManager {
            bindings: GenerationalArena::new(),
            mailbox_manager: MailboxManager::new(),
            connections: GenerationalArena::new(),
            registry: BTreeMap::new(),
        }
    }

    pub(crate) fn bind_service(&mut self, service: &str) -> Result<IpcBindingHandle, IpcBindingError> {
        if self.registry.contains_key(service) {
           return Err(IpcBindingError::AlreadyBound);
        }

        let mailbox_handle = self.mailbox_manager.create();
        let binding = IpcServerBinding::new(String::from(service), mailbox_handle);
        let binding_handle = self.bindings.add(binding).unwrap();
        self.registry.insert(String::from(service), binding_handle);

        Ok(binding_handle)
    }

    pub(crate) fn connect(&mut self, service: &str) -> Result<IpcConnectionHandle, IpcConnectionError> {
        if let Some(binding_handler) = self.registry.get(service).copied() {
            if let Ok(server_binding) = self.bindings.borrow(binding_handler) {
                let client_mailbox_handle = self.mailbox_manager.create();
                let server_mailbox_handle = server_binding.mailbox_handle;
                let connection = IpcConnection {
                    server_mailbox: server_mailbox_handle,
                    client_mailbox: client_mailbox_handle
                };
                if let Ok(connection_handle) = self.connections.add(connection) {
                    Ok(connection_handle)
                } else {
                    Err(IpcConnectionError::ConnectionCannotBeEstablished)
                }
            } else {
                Err(IpcConnectionError::ServerNotFound)
            }
        } else {
            Err(IpcConnectionError::ServerNotFound)
        }
    }

    pub(crate) fn disconnect(&mut self, connection_handle: IpcConnectionHandle) {
        if let Ok(connection) = self.connections.remove(connection_handle) {
            self.mailbox_manager.remove(connection.client_mailbox);
        }
    }

    pub(crate) fn send_to_server(&mut self, message: IpcMessage) -> Result<(), IpcSendError> {
        let connection_handle = message.connection_handle;
        if let Ok(connection) = self.connections.borrow(connection_handle) {
            self.mailbox_manager.push_back(connection.server_mailbox, message);
            Ok(())
        } else {
            Err(IpcSendError::ConnectionNotFound)
        }
    }

    pub(crate) fn send_to_client(&mut self, message: IpcMessage) -> Result<(), IpcSendError> {
        let connection_handle = message.connection_handle;
        if let Ok(connection) = self.connections.borrow(connection_handle) {
            self.mailbox_manager.push_back(connection.client_mailbox, message);
            Ok(())
        } else {
            Err(IpcSendError::ConnectionNotFound)
        }
    }

    pub(crate) fn receive_from_all_clients_async(&mut self, server_binding_handle: IpcBindingHandle) -> IpcMessageFuture {
        if let Ok(server_binding) = self.bindings.borrow(server_binding_handle) {
            let server_mailbox_handle = server_binding.mailbox_handle;
            self.mailbox_manager.pop_front_async(server_mailbox_handle)
        } else {
            IpcMessageFuture::with_error(IpcReceiveError::ConnectionNotFound)
        }
    }

    pub(crate) fn receive_from_server_async(&mut self, connection_handle: IpcConnectionHandle) -> IpcMessageFuture {
        if let Ok(connection) = self.connections.borrow(connection_handle) {
            self.mailbox_manager.pop_front_async(connection.client_mailbox)
        } else {
            IpcMessageFuture::with_error(IpcReceiveError::ConnectionNotFound)
        }
    }
}
