use alloc::collections::{BTreeMap};
use alloc::string::String;
use alloc::boxed::Box;
use collections::generational_arena::{GenerationalArena, Handle};
use system::ipc::{IpcConnectionError, IpcMessage, IpcSendError, IpcConnectionHandle, IpcBindingError, IpcReceiveError, IpcMessageFuture, IpcMessageError, IpcMessageHandle};
use system::future::FutureHandle;
use crate::ipc::mailbox_manager::{MailboxManager, MailboxHandle};
use crate::ipc::message_buffer::MessageRoute;
use crate::ipc::message_buffer_manager::MessageBufferManager;
use crate::ports::driven::ForNotifyingFutures;
use crate::task::TaskHandle;

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
    client_task: TaskHandle,
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

    pub(crate) fn connect(&mut self, service: &str, caller: TaskHandle) -> Result<IpcConnectionHandle, IpcConnectionError> {
        if let Some(binding_handler) = self.registry.get(service).copied() {
            if let Ok(server_binding) = self.bindings.borrow(binding_handler) {
                let client_mailbox_handle = self.mailbox_manager.create();
                let server_mailbox_handle = server_binding.mailbox_handle;
                let connection = IpcConnection {
                    server_mailbox: server_mailbox_handle,
                    client_mailbox: client_mailbox_handle,
                    client_task: caller
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
        let client_mailbox = match self.connections.borrow(connection_handle) {
            Ok(connection) => connection.client_mailbox,
            Err(_) => return,
        };
        for message in self.mailbox_manager.drain(client_mailbox) {
            let _ = self.buffer_manager.dispose(message.message_handle);
        }
        self.mailbox_manager.remove(client_mailbox);
        let _ = self.connections.remove(connection_handle);
    }

    pub(crate) fn create_message(&mut self, connection: IpcConnectionHandle, caller: TaskHandle, data_size: usize) -> Result<IpcMessageHandle, IpcSendError> {
        let destination = match self.connections.borrow(connection) {
            Ok(conn) => {
                if caller == conn.client_task {
                    conn.server_mailbox
                } else {
                    conn.client_mailbox
                }
            }
            Err(_) => return Err(IpcSendError::ConnectionNotFound),
        };
        self.buffer_manager
            .create(data_size, MessageRoute { connection, destination })
            .map_err(IpcSendError::InvalidBuffer)
    }

    pub(crate) fn send_message(&mut self, message_handle: IpcMessageHandle) -> Result<(), IpcSendError> {
        let route = match self.buffer_manager.route_of(message_handle) {
            Some(route) => route,
            None => return Err(IpcSendError::InvalidBuffer(IpcMessageError::BufferNotFound)),
        };
        if self.connections.borrow(route.connection).is_err() {
            return Err(IpcSendError::ConnectionNotFound);
        }
        self.buffer_manager.seal(message_handle).map_err(IpcSendError::InvalidBuffer)?;
        self.mailbox_manager.push_back(route.destination, IpcMessage {
            message_handle,
            connection_handle: route.connection,
        });
        Ok(())
    }

    pub(crate) fn write_message(&mut self, handle: IpcMessageHandle, bytes: &[u8]) -> Result<(), IpcMessageError> {
        self.buffer_manager.write(handle, bytes)
    }

    pub(crate) fn read_message(&mut self, handle: IpcMessageHandle, dst: &mut [u8]) -> Result<usize, IpcMessageError> {
        self.buffer_manager.read(handle, dst)
    }

    pub(crate) fn dispose_message(&mut self, handle: IpcMessageHandle) -> Result<(), IpcMessageError> {
        self.buffer_manager.dispose(handle)
    }

    pub(crate) fn accept_message_async(&mut self, server_binding_handle: IpcBindingHandle) -> FutureHandle {
        if let Ok(server_binding) = self.bindings.borrow(server_binding_handle) {
            let server_mailbox_handle = server_binding.mailbox_handle;
            self.mailbox_manager.pop_front_async(server_mailbox_handle)
        } else {
            self.notifier
                .register(Box::new(IpcMessageFuture::with_error(IpcReceiveError::ConnectionNotFound)))
                .unwrap()
        }
    }

    pub(crate) fn receive_message_async(&mut self, connection_handle: IpcConnectionHandle) -> FutureHandle {
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

    const CLIENT_TASK: Handle = Handle { index: 1, generation: 1 };
    const SERVER_TASK: Handle = Handle { index: 2, generation: 1 };

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
        let connection = manager.connect(service, CLIENT_TASK).unwrap();
        (binding, connection)
    }

    #[test]
    fn client_send_accept_yields_sealed_message() {
        let mut manager = manager_with_notifier();
        let (binding, connection) = bind_and_connect(&mut manager, "MSG-E2E");

        let message_handle = manager.create_message(connection, CLIENT_TASK, 8).unwrap();
        assert!(manager.write_message(message_handle, b"payload").is_ok());
        assert!(manager.send_message(message_handle).is_ok());

        let fh = manager.accept_message_async(binding);
        {
            let registry = services().future_registry.borrow_mut();
            let future_box = registry.borrow_mut(fh).unwrap();
            let ipc_future = future_box.as_any().downcast_ref::<IpcMessageFuture>().unwrap();
            let delivered = ipc_future.result().unwrap();
            assert_eq!(delivered.message_handle, message_handle);
            assert_eq!(delivered.connection_handle, connection);
        }

        let mut dst = [0u8; 64];
        let copied = manager.read_message(message_handle, &mut dst).unwrap();
        assert_eq!(copied, 7);
        assert_eq!(&dst[..copied], b"payload");
    }

    #[test]
    fn server_reply_reaches_client_mailbox() {
        let mut manager = manager_with_notifier();
        let (_binding, connection) = bind_and_connect(&mut manager, "MSG-REPLY");

        let reply_handle = manager.create_message(connection, SERVER_TASK, 8).unwrap();
        assert!(manager.write_message(reply_handle, b"reply!!").is_ok());
        assert!(manager.send_message(reply_handle).is_ok());

        let fh = manager.receive_message_async(connection);
        {
            let registry = services().future_registry.borrow_mut();
            let future_box = registry.borrow_mut(fh).unwrap();
            let ipc_future = future_box.as_any().downcast_ref::<IpcMessageFuture>().unwrap();
            let delivered = ipc_future.result().unwrap();
            assert_eq!(delivered.message_handle, reply_handle);
            assert_eq!(delivered.connection_handle, connection);
        }
    }

    #[test]
    fn double_send_of_same_message_is_rejected() {
        let mut manager = manager_with_notifier();
        let (_binding, connection) = bind_and_connect(&mut manager, "MSG-DOUBLE");

        let message_handle = manager.create_message(connection, CLIENT_TASK, 8).unwrap();
        assert!(manager.write_message(message_handle, b"once").is_ok());
        assert!(manager.send_message(message_handle).is_ok());

        assert_eq!(
            manager.send_message(message_handle),
            Err(IpcSendError::InvalidBuffer(IpcMessageError::Sealed))
        );
    }

    #[test]
    fn send_of_disposed_message_is_rejected() {
        let mut manager = manager_with_notifier();
        let (_binding, connection) = bind_and_connect(&mut manager, "MSG-DISPOSED");

        let message_handle = manager.create_message(connection, CLIENT_TASK, 8).unwrap();
        assert!(manager.dispose_message(message_handle).is_ok());

        assert_eq!(
            manager.send_message(message_handle),
            Err(IpcSendError::InvalidBuffer(IpcMessageError::BufferNotFound))
        );
    }

    #[test]
    fn send_on_dead_connection_skips_seal() {
        let mut manager = manager_with_notifier();
        let (_binding, connection) = bind_and_connect(&mut manager, "MSG-DEADCONN");

        let message_handle = manager.create_message(connection, CLIENT_TASK, 8).unwrap();
        assert!(manager.write_message(message_handle, b"orphan").is_ok());
        manager.disconnect(connection);

        assert_eq!(
            manager.send_message(message_handle),
            Err(IpcSendError::ConnectionNotFound)
        );
        assert_eq!(manager.read_message(message_handle, &mut [0u8; 8]), Err(IpcMessageError::Unsealed));
        assert!(manager.dispose_message(message_handle).is_ok());
    }

    #[test]
    fn create_message_on_dead_connection_is_rejected() {
        let mut manager = manager_with_notifier();
        let (_binding, connection) = bind_and_connect(&mut manager, "MSG-DEADCREATE");
        manager.disconnect(connection);

        assert_eq!(
            manager.create_message(connection, CLIENT_TASK, 8),
            Err(IpcSendError::ConnectionNotFound)
        );
    }

    #[test]
    fn create_message_rejects_data_size_above_cap() {
        let mut manager = manager_with_notifier();
        let (_binding, connection) = bind_and_connect(&mut manager, "MSG-TOOBIG");

        assert_eq!(
            manager.create_message(connection, CLIENT_TASK, 65),
            Err(IpcSendError::InvalidBuffer(IpcMessageError::MessageTooLarge))
        );
    }

    #[test]
    fn write_message_enforces_cap() {
        let mut manager = manager_with_notifier();
        let (_binding, connection) = bind_and_connect(&mut manager, "MSG-CAP");

        let message_handle = manager.create_message(connection, CLIENT_TASK, 64).unwrap();
        assert!(manager.write_message(message_handle, &[0xAA; 64]).is_ok());
        assert_eq!(
            manager.write_message(message_handle, &[0xBB; 1]),
            Err(IpcMessageError::MessageTooLarge)
        );
    }

    #[test]
    fn accept_before_connect_delivers_message() {
        use system::future::Future;
        let mut manager = manager_with_notifier();
        let binding = manager.bind_service("MSG-EARLY").unwrap();

        let fh = manager.accept_message_async(binding);
        {
            let registry = services().future_registry.borrow_mut();
            let future_box = registry.borrow_mut(fh).unwrap();
            let ipc_future = future_box.as_any().downcast_ref::<IpcMessageFuture>().unwrap();
            assert!(!ipc_future.is_completed());
        }

        let connection = manager.connect("MSG-EARLY", CLIENT_TASK).unwrap();
        let message_handle = manager.create_message(connection, CLIENT_TASK, 8).unwrap();
        assert!(manager.send_message(message_handle).is_ok());

        {
            let registry = services().future_registry.borrow_mut();
            let future_box = registry.borrow_mut(fh).unwrap();
            let ipc_future = future_box.as_any().downcast_ref::<IpcMessageFuture>().unwrap();
            assert_eq!(ipc_future.result().unwrap().message_handle, message_handle);
        }
    }

    #[test]
    fn receive_on_dead_connection_yields_error_future() {
        use system::future::Future;
        let mut manager = manager_with_notifier();
        let (_binding, connection) = bind_and_connect(&mut manager, "MSG-DEADRECV");
        manager.disconnect(connection);

        let fh = manager.receive_message_async(connection);
        let registry = services().future_registry.borrow_mut();
        let future_box = registry.borrow_mut(fh).unwrap();
        let ipc_future = future_box.as_any().downcast_ref::<IpcMessageFuture>().unwrap();
        assert!(ipc_future.is_completed());
        assert!(matches!(
            ipc_future.result(),
            Err(IpcReceiveError::ConnectionNotFound)
        ));
    }

    #[test]
    fn accept_on_unknown_binding_yields_error_future() {
        use system::future::Future;
        let mut manager = manager_with_notifier();

        let fh = manager.accept_message_async(Handle::new(99, 1));
        let registry = services().future_registry.borrow_mut();
        let future_box = registry.borrow_mut(fh).unwrap();
        let ipc_future = future_box.as_any().downcast_ref::<IpcMessageFuture>().unwrap();
        assert!(ipc_future.is_completed());
        assert!(matches!(
            ipc_future.result(),
            Err(IpcReceiveError::ConnectionNotFound)
        ));
    }

    #[test]
    fn disconnect_disposes_messages_queued_for_client() {
        let mut manager = manager_with_notifier();
        let (_binding, connection) = bind_and_connect(&mut manager, "MSG-DRAIN");

        let reply_handle = manager.create_message(connection, SERVER_TASK, 8).unwrap();
        assert!(manager.send_message(reply_handle).is_ok());

        manager.disconnect(connection);

        assert_eq!(
            manager.read_message(reply_handle, &mut [0u8; 8]),
            Err(IpcMessageError::BufferNotFound)
        );
    }
}
