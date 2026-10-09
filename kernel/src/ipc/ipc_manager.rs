use alloc::collections::{BTreeMap};
use alloc::string::String;
use alloc::boxed::Box;
use collections::generational_arena::{GenerationalArena, Handle};
use system::ipc::{IpcConnectionError, IpcMessage, IpcSendError, IpcConnectionHandle, IpcBindingError, IpcReceiveError, IpcMessageFuture, IpcBufferError, IpcBufferHandle};
use system::future::FutureHandle;
use crate::ipc::mailbox_manager::{MailboxManager, MailboxHandle};
use crate::ipc::message_buffer_manager::MessageBufferManager;
use crate::ports::driven::ForNotifyingFutures;

struct NoopIpcNotifier;
impl ForNotifyingFutures for NoopIpcNotifier {
    fn register(&self, _future: Box<dyn system::future::Future + Send + Sync>) -> Option<FutureHandle> {
        None
    }
    fn notify(&self, _handle: FutureHandle) {}
    fn complete_ipc_message(&self, _handle: FutureHandle, _message: IpcMessage) {}
}

static NOOP_IPC_NOTIFIER: NoopIpcNotifier = NoopIpcNotifier;

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
    buffer_manager: MessageBufferManager,
    connections: GenerationalArena<IpcConnection, 256>,
    registry: BTreeMap<String, IpcBindingHandle>,
    notifier: &'static dyn ForNotifyingFutures,
}

impl IpcManager {

    pub(crate) fn new() -> IpcManager {
        IpcManager {
            bindings: GenerationalArena::new(),
            mailbox_manager: MailboxManager::new(),
            buffer_manager: MessageBufferManager::new(),
            connections: GenerationalArena::new(),
            registry: BTreeMap::new(),
            notifier: &NOOP_IPC_NOTIFIER,
        }
    }

    pub(crate) fn new_with_notifier(notifier: &'static dyn ForNotifyingFutures) -> IpcManager {
        IpcManager {
            bindings: GenerationalArena::new(),
            mailbox_manager: MailboxManager::new_with_notifier(notifier),
            buffer_manager: MessageBufferManager::new(),
            connections: GenerationalArena::new(),
            registry: BTreeMap::new(),
            notifier,
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
        let server_mailbox = match self.connections.borrow(connection_handle) {
            Ok(connection) => connection.server_mailbox,
            Err(_) => return Err(IpcSendError::ConnectionNotFound),
        };
        self.buffer_manager.seal(message.buffer_handle).map_err(IpcSendError::InvalidBuffer)?;
        self.mailbox_manager.push_back(server_mailbox, message);
        Ok(())
    }

    pub(crate) fn send_to_client(&mut self, message: IpcMessage) -> Result<(), IpcSendError> {
        let connection_handle = message.connection_handle;
        let client_mailbox = match self.connections.borrow(connection_handle) {
            Ok(connection) => connection.client_mailbox,
            Err(_) => return Err(IpcSendError::ConnectionNotFound),
        };
        self.buffer_manager.seal(message.buffer_handle).map_err(IpcSendError::InvalidBuffer)?;
        self.mailbox_manager.push_back(client_mailbox, message);
        Ok(())
    }

    pub(crate) fn alloc_buffer(&mut self) -> Result<IpcBufferHandle, IpcBufferError> {
        self.buffer_manager.alloc()
    }

    pub(crate) fn write_buffer(&mut self, handle: IpcBufferHandle, bytes: &[u8]) -> Result<(), IpcBufferError> {
        self.buffer_manager.write(handle, bytes)
    }

    pub(crate) fn read_buffer(&mut self, handle: IpcBufferHandle, dst: &mut [u8]) -> Result<usize, IpcBufferError> {
        self.buffer_manager.read(handle, dst)
    }

    pub(crate) fn dispose_buffer(&mut self, handle: IpcBufferHandle) -> Result<(), IpcBufferError> {
        self.buffer_manager.dispose(handle)
    }

    pub(crate) fn receive_from_all_clients_async(&mut self, server_binding_handle: IpcBindingHandle) -> FutureHandle {
        if let Ok(server_binding) = self.bindings.borrow(server_binding_handle) {
            let server_mailbox_handle = server_binding.mailbox_handle;
            self.mailbox_manager.pop_front_async(server_mailbox_handle)
        } else {
            self.notifier
                .register(Box::new(IpcMessageFuture::with_error(IpcReceiveError::ConnectionNotFound)))
                .unwrap()
        }
    }

    pub(crate) fn receive_from_server_async(&mut self, connection_handle: IpcConnectionHandle) -> FutureHandle {
        if let Ok(connection) = self.connections.borrow(connection_handle) {
            self.mailbox_manager.pop_front_async(connection.client_mailbox)
        } else {
            self.notifier
                .register(Box::new(IpcMessageFuture::with_error(IpcReceiveError::ConnectionNotFound)))
                .unwrap()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel_services::init as init_services;
    use crate::kernel_services::services;
    use crate::kernel_services::FutureRegistryNotifierUseCase;
    use crate::ports::driven::ForNotifyingFutures;

    fn manager_with_notifier() -> IpcManager {
        init_services();
        let notifier: &'static dyn ForNotifyingFutures =
            Box::leak(Box::new(FutureRegistryNotifierUseCase {
                future_registry: services().future_registry,
            })) as &'static dyn ForNotifyingFutures;
        IpcManager::new_with_notifier(notifier)
    }

    fn bind_and_connect(manager: &mut IpcManager, service: &str) -> (IpcBindingHandle, IpcConnectionHandle) {
        let binding = manager.bind_service(service).unwrap();
        let connection = manager.connect(service).unwrap();
        (binding, connection)
    }

    #[test]
    fn send_to_server_seals_buffer_and_delivers_handle() {
        let mut manager = manager_with_notifier();
        let (binding, connection) = bind_and_connect(&mut manager, "SEAL-E2E");

        let buffer_handle = manager.alloc_buffer().unwrap();
        assert!(manager.write_buffer(buffer_handle, b"payload").is_ok());
        assert!(manager.send_to_server(IpcMessage { buffer_handle, connection_handle: connection }).is_ok());

        let fh = manager.receive_from_all_clients_async(binding);
        {
            let mut registry = services().future_registry.borrow_mut();
            let future_box = registry.borrow_mut(fh).unwrap();
            let ipc_future = future_box.as_any().downcast_ref::<IpcMessageFuture>().unwrap();
            assert_eq!(ipc_future.result().unwrap().buffer_handle, buffer_handle);
        }

        let mut dst = [0u8; 64];
        let copied = manager.read_buffer(buffer_handle, &mut dst).unwrap();
        assert_eq!(copied, 7);
        assert_eq!(&dst[..copied], b"payload");
    }

    #[test]
    fn double_send_of_same_buffer_is_rejected() {
        let mut manager = manager_with_notifier();
        let (_binding, connection) = bind_and_connect(&mut manager, "SEAL-DOUBLE");

        let buffer_handle = manager.alloc_buffer().unwrap();
        assert!(manager.write_buffer(buffer_handle, b"once").is_ok());
        assert!(manager.send_to_server(IpcMessage { buffer_handle, connection_handle: connection }).is_ok());

        assert_eq!(
            manager.send_to_server(IpcMessage { buffer_handle, connection_handle: connection }),
            Err(IpcSendError::InvalidBuffer(IpcBufferError::Sealed))
        );
    }

    #[test]
    fn send_of_disposed_buffer_is_rejected() {
        let mut manager = manager_with_notifier();
        let (_binding, connection) = bind_and_connect(&mut manager, "SEAL-DISPOSED");

        let buffer_handle = manager.alloc_buffer().unwrap();
        assert!(manager.dispose_buffer(buffer_handle).is_ok());

        assert_eq!(
            manager.send_to_server(IpcMessage { buffer_handle, connection_handle: connection }),
            Err(IpcSendError::InvalidBuffer(IpcBufferError::BufferNotFound))
        );
    }

    #[test]
    fn send_on_dead_connection_skips_seal() {
        let mut manager = manager_with_notifier();
        let (_binding, connection) = bind_and_connect(&mut manager, "SEAL-DEADCONN");

        let buffer_handle = manager.alloc_buffer().unwrap();
        assert!(manager.write_buffer(buffer_handle, b"orphan").is_ok());
        manager.disconnect(connection);

        assert_eq!(
            manager.send_to_server(IpcMessage { buffer_handle, connection_handle: connection }),
            Err(IpcSendError::ConnectionNotFound)
        );
        assert_eq!(manager.read_buffer(buffer_handle, &mut [0u8; 8]), Err(IpcBufferError::Unsealed));
        assert!(manager.dispose_buffer(buffer_handle).is_ok());
    }
}
