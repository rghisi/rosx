use alloc::collections::{BTreeMap, VecDeque};
use alloc::string::String;
use collections::generational_arena::{GenerationalArena, Handle};
use system::ipc::{IpcConnectionError, IpcMessage, IpcSendError, IpcConnectionHandle, IpcBindingError, IpcReceiveError};
use crate::ipc::mailbox::Mailbox;

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
type MailboxHandle = Handle;

struct IpcConnection {
    server_mailbox: MailboxHandle,
    client_mailbox: MailboxHandle,
}

pub(crate) struct IpcManager {
    bindings: GenerationalArena<IpcServerBinding, 256>,
    mailboxes: GenerationalArena<Mailbox, 256>,
    connections: GenerationalArena<IpcConnection, 256>,
    registry: BTreeMap<String, IpcBindingHandle>,
}

impl IpcManager {

    pub(crate) fn new() -> IpcManager {
        IpcManager {
            bindings: GenerationalArena::new(),
            mailboxes: GenerationalArena::new(),
            connections: GenerationalArena::new(),
            registry: BTreeMap::new(),
        }
    }

    pub(crate) fn bind_service(&mut self, service: &str) -> Result<IpcBindingHandle, IpcBindingError> {
        if self.registry.contains_key(service) {
           return Err(IpcBindingError::AlreadyBound);
        }

        let mailbox_handle = self.mailboxes.add(Mailbox::new()).unwrap();
        let binding = IpcServerBinding::new(String::from(service), mailbox_handle);
        let binding_handle = self.bindings.add(binding).unwrap();
        self.registry.insert(String::from(service), binding_handle);

        Ok(binding_handle)
    }

    pub(crate) fn connect(&mut self, service: &str) -> Result<IpcConnectionHandle, IpcConnectionError> {
        if let Some(binding_handler) = self.registry.get(service).copied() {
            if let Ok(server_biding) = self.bindings.borrow(binding_handler) {
                if let Ok(client_mailbox_handle) = self.mailboxes.add(Mailbox::new()) {
                    let server_mailbox_handle = server_biding.mailbox_handle;
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
                    Err(IpcConnectionError::ConnectionCannotBeEstablished)
                }
            } else {
                Err(IpcConnectionError::ServerNotFound)
            }
        } else {
            Err(IpcConnectionError::ServerNotFound)
        }
    }

    pub(crate) fn send(&mut self, message: IpcMessage) -> Result<(), IpcSendError> {
        let connection_handle = message.connection_handle;
        if let Ok(connection) = self.connections.borrow(connection_handle) {
            if let Ok(mailbox) = self.mailboxes.borrow_mut(connection.server_mailbox) {
                mailbox.push_back(message);
                Ok(())
            } else {
                Err(IpcSendError::ConnectionNotFound)
            }
        } else {
            Err(IpcSendError::ConnectionNotFound)
        }
    }

    pub(crate) fn reply(&mut self, message: IpcMessage) -> Result<(), IpcSendError> {
        let connection_handle = message.connection_handle;
        if let Ok(connection) = self.connections.borrow(connection_handle) {
            if let Ok(mailbox) = self.mailboxes.borrow_mut(connection.client_mailbox) {
                mailbox.push_back(message);
                Ok(())
            } else {
                Err(IpcSendError::ConnectionNotFound)
            }
        } else {
            Err(IpcSendError::ConnectionNotFound)
        }
    }

    pub(crate) fn receive_from_binding(&mut self, server_binding_handle: IpcBindingHandle) -> Result<IpcMessage, IpcReceiveError> {
        if let Ok(server_binding) = self.bindings.borrow(server_binding_handle) {
            let server_mailbox_handle = server_binding.mailbox_handle;
            if let Ok(server_mailbox) =self.mailboxes.borrow_mut(server_mailbox_handle) {
                if let Some(message) = server_mailbox.pop_front() {
                    Ok(message)
                } else {
                    Err(IpcReceiveError::NoMessagesAvailable)
                }
            } else {
                Err(IpcReceiveError::ConnectionNotFound)
            }
        } else {
            Err(IpcReceiveError::ConnectionNotFound)
        }
    }

    pub(crate) fn receive(&mut self, connection_handle: IpcConnectionHandle) -> Result<IpcMessage, IpcReceiveError> {
        if let Ok(connection) = self.connections.borrow(connection_handle) {
            if let Ok(client_mailbox) = self.mailboxes.borrow_mut(connection.client_mailbox) {
                if let Some(message) = client_mailbox.pop_front() {
                    Ok(message)
                } else {
                    Err(IpcReceiveError::NoMessagesAvailable)
                }
            } else {
                Err(IpcReceiveError::ConnectionNotFound)
            }
        } else {
            Err(IpcReceiveError::ConnectionNotFound)
        }
    }
}