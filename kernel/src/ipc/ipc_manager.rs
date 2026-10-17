use alloc::collections::{BTreeMap};
use alloc::string::String;
use alloc::boxed::Box;
use crate::collections::arena::GenerationalArena;
use crate::future::Future;
use crate::handle::{FutureHandle, IpcBindingHandle, IpcConnectionHandle, IpcDataHandle};
use crate::ipc::data_manager::IpcDataManager;
use crate::ipc::errors::{DataError, IpcBindingError, IpcConnectionError, IpcReceiveError, IpcSendError};
use crate::ipc::message_future::IpcMessageFuture;
use system::ipc::Message;
use crate::ipc::mailbox_manager::{MailboxManager, MailboxHandle};
use crate::ports::driven::ForNotifyingFutures;
use crate::task::TaskHandle;

struct NoopIpcNotifier;
impl ForNotifyingFutures for NoopIpcNotifier {
    fn register(&self, _future: Box<dyn Future + Send + Sync>) -> Option<FutureHandle> {
        None
    }
    fn notify(&self, _handle: FutureHandle) {}
    fn complete_ipc_message(&self, _handle: FutureHandle, _message: Message) {}
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

struct IpcConnection {
    server_mailbox: MailboxHandle,
    client_mailbox: MailboxHandle,
    client_task: TaskHandle,
}

pub(crate) struct IpcManager {
    bindings: GenerationalArena<IpcServerBinding, 256>,
    mailbox_manager: MailboxManager,
    connections: GenerationalArena<IpcConnection, 256>,
    registry: BTreeMap<String, IpcBindingHandle>,
    notifier: &'static dyn ForNotifyingFutures,
    data_manager: IpcDataManager,
}

impl IpcManager {

    pub(crate) fn new() -> IpcManager {
        IpcManager {
            bindings: GenerationalArena::new(),
            mailbox_manager: MailboxManager::new(),
            connections: GenerationalArena::new(),
            registry: BTreeMap::new(),
            notifier: &NOOP_IPC_NOTIFIER,
            data_manager: IpcDataManager::new(),
        }
    }

    pub(crate) fn new_with_notifier(notifier: &'static dyn ForNotifyingFutures) -> IpcManager {
        IpcManager {
            bindings: GenerationalArena::new(),
            mailbox_manager: MailboxManager::new_with_notifier(notifier),
            connections: GenerationalArena::new(),
            registry: BTreeMap::new(),
            notifier,
            data_manager: IpcDataManager::new(),
        }
    }

    pub(crate) fn bind_service(&mut self, service: &str) -> Result<IpcBindingHandle, IpcBindingError> {
        if self.registry.contains_key(service) {
           return Err(IpcBindingError::AlreadyBound);
        }

        let mailbox_handle = self.mailbox_manager.create();
        let binding = IpcServerBinding::new(String::from(service), mailbox_handle);
        let binding_handle = IpcBindingHandle::from(self.bindings.add(binding).unwrap());
        self.registry.insert(String::from(service), binding_handle);

        Ok(binding_handle)
    }

    pub(crate) fn connect(&mut self, service: &str, caller: TaskHandle) -> Result<IpcConnectionHandle, IpcConnectionError> {
        if let Some(binding_handler) = self.registry.get(service).copied() {
            if let Ok(server_binding) = self.bindings.borrow(binding_handler.into()) {
                let client_mailbox_handle = self.mailbox_manager.create();
                let server_mailbox_handle = server_binding.mailbox_handle;
                let connection = IpcConnection {
                    server_mailbox: server_mailbox_handle,
                    client_mailbox: client_mailbox_handle,
                    client_task: caller
                };
                if let Ok(connection_handle) = self.connections.add(connection) {
                    Ok(IpcConnectionHandle::from(connection_handle))
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
        let client_mailbox = match self.connections.borrow(connection_handle.into()) {
            Ok(connection) => connection.client_mailbox,
            Err(_) => return,
        };
        self.mailbox_manager.remove(client_mailbox);
        let _ = self.connections.remove(connection_handle.into());
        self.data_manager.gc_connection(connection_handle);
    }

    pub(crate) fn send_message(&mut self, caller: TaskHandle, envelope: &Message) -> Result<(), IpcSendError> {
        let connection = match IpcConnectionHandle::from_packed(envelope.conn() as usize) {
            Some(connection) => connection,
            None => return Err(IpcSendError::ConnectionNotFound),
        };
        let destination = match self.connections.borrow(connection.into()) {
            Ok(conn) => {
                if caller == conn.client_task {
                    conn.server_mailbox
                } else {
                    conn.client_mailbox
                }
            }
            Err(_) => return Err(IpcSendError::ConnectionNotFound),
        };
        self.mailbox_manager.push_back(destination, *envelope)
    }

    pub(crate) fn accept_message_async(&mut self, server_binding_handle: IpcBindingHandle) -> FutureHandle {
        if let Ok(server_binding) = self.bindings.borrow(server_binding_handle.into()) {
            let server_mailbox_handle = server_binding.mailbox_handle;
            self.mailbox_manager.pop_front_async(server_mailbox_handle)
        } else {
            self.notifier
                .register(Box::new(IpcMessageFuture::with_error(IpcReceiveError::ConnectionNotFound)))
                .unwrap()
        }
    }

    pub(crate) fn receive_message_async(&mut self, connection_handle: IpcConnectionHandle) -> FutureHandle {
        if let Ok(connection) = self.connections.borrow(connection_handle.into()) {
            self.mailbox_manager.pop_front_async(connection.client_mailbox)
        } else {
            self.notifier
                .register(Box::new(IpcMessageFuture::with_error(IpcReceiveError::ConnectionNotFound)))
                .unwrap()
        }
    }
}

#[allow(dead_code)]
impl IpcManager {

    pub(crate) fn write_data(
        &mut self,
        caller: TaskHandle,
        connection: IpcConnectionHandle,
        payload: &[u8],
    ) -> Result<IpcDataHandle, DataError> {
        self.connections
            .borrow(connection.into())
            .map_err(|_| DataError::ConnectionNotFound)?;
        self.data_manager.write(caller, connection, payload)
    }

    pub(crate) fn read_data(
        &mut self,
        caller: TaskHandle,
        token: IpcDataHandle,
        dst: &mut [u8],
    ) -> Result<usize, DataError> {
        let (connection, writer) = self.data_manager.locate(caller, token)?;
        let conn = self
            .connections
            .borrow(connection.into())
            .map_err(|_| DataError::NotReadable)?;
        if caller != conn.client_task && writer != conn.client_task {
            return Err(DataError::NotReadable);
        }
        Ok(self.data_manager.copy_out(token, dst))
    }

    pub(crate) fn release_data(
        &mut self,
        caller: TaskHandle,
        token: IpcDataHandle,
    ) -> Result<(), DataError> {
        self.data_manager.release(caller, token)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel_services::init as init_services;
    use crate::kernel_services::services;
    use crate::kernel_services::FutureRegistryNotifierUseCase;
    use crate::ports::driven::ForNotifyingFutures;
    use alloc::vec;
    use system::ipc::MESSAGE_PAYLOAD_BYTES;

    const CLIENT_TASK: TaskHandle = TaskHandle::new(1, 1);
    const SERVER_TASK: TaskHandle = TaskHandle::new(2, 1);
    const INTRUDER: TaskHandle = TaskHandle::new(3, 1);

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

    fn delivered_message(fh: FutureHandle) -> Message {
        let registry = services().future_registry.borrow_mut();
        let future_box = registry.borrow_mut(fh).unwrap();
        let ipc_future = future_box.as_any().downcast_ref::<IpcMessageFuture>().unwrap();
        ipc_future.result().unwrap()
    }

    #[test]
    fn client_send_accept_delivers_envelope() {
        let mut manager = manager_with_notifier();
        let (binding, connection) = bind_and_connect(&mut manager, "MSG-E2E");

        let pattern: [u8; MESSAGE_PAYLOAD_BYTES] = core::array::from_fn(|i| (i * 7 + 3) as u8);
        assert_eq!(manager.send_message(CLIENT_TASK, &Message::new(connection.pack() as u32, &pattern)), Ok(()));

        let fh = manager.accept_message_async(binding);
        let delivered = delivered_message(fh);
        assert_eq!(delivered.conn(), connection.pack() as u32);
        assert_eq!(delivered.data(), &pattern);
    }

    #[test]
    fn server_reply_reaches_client_mailbox() {
        let mut manager = manager_with_notifier();
        let (binding, connection) = bind_and_connect(&mut manager, "MSG-REPLY");

        let request_pattern = [0x11u8; MESSAGE_PAYLOAD_BYTES];
        assert_eq!(manager.send_message(CLIENT_TASK, &Message::new(connection.pack() as u32, &request_pattern)), Ok(()));
        let fh = manager.accept_message_async(binding);
        let delivered = delivered_message(fh);
        assert_eq!(delivered.conn(), connection.pack() as u32);

        let reply_pattern = [0x22u8; MESSAGE_PAYLOAD_BYTES];
        assert_eq!(manager.send_message(SERVER_TASK, &Message::new(delivered.conn(), &reply_pattern)), Ok(()));

        let fh = manager.receive_message_async(connection);
        let reply = delivered_message(fh);
        assert_eq!(reply.conn(), connection.pack() as u32);
        assert_eq!(reply.data(), &reply_pattern);
    }

    #[test]
    fn send_on_stale_connection_rejected() {
        let mut manager = manager_with_notifier();
        let (_binding, connection) = bind_and_connect(&mut manager, "MSG-STALE");
        manager.disconnect(connection);

        assert_eq!(
            manager.send_message(CLIENT_TASK, &Message::new(connection.pack() as u32, &[0u8; MESSAGE_PAYLOAD_BYTES])),
            Err(IpcSendError::ConnectionNotFound)
        );
    }

    #[test]
    fn send_on_recycled_connection_slot_rejected() {
        let mut manager = manager_with_notifier();
        let (_binding, c1) = bind_and_connect(&mut manager, "MSG-ABA");
        manager.disconnect(c1);

        for _ in 0..255 {
            let filler = manager.connect("MSG-ABA", CLIENT_TASK).unwrap();
            manager.disconnect(filler);
        }

        let c2 = manager.connect("MSG-ABA", CLIENT_TASK).unwrap();
        assert_eq!(c2.index(), c1.index());
        assert_eq!(c2.generation(), c1.generation() + 1);

        assert_eq!(
            manager.send_message(CLIENT_TASK, &Message::new(c1.pack() as u32, &[0u8; MESSAGE_PAYLOAD_BYTES])),
            Err(IpcSendError::ConnectionNotFound)
        );
        assert_eq!(
            manager.send_message(CLIENT_TASK, &Message::new(c2.pack() as u32, &[0u8; MESSAGE_PAYLOAD_BYTES])),
            Ok(())
        );
    }

    #[test]
    fn send_congested_at_32_and_recovers_after_receive() {
        let mut manager = manager_with_notifier();
        let (binding, connection) = bind_and_connect(&mut manager, "MSG-FLOOD");

        let envelope = Message::new(connection.pack() as u32, &[0u8; MESSAGE_PAYLOAD_BYTES]);
        for _ in 0..32 {
            assert_eq!(manager.send_message(CLIENT_TASK, &envelope), Ok(()));
        }
        assert_eq!(manager.send_message(CLIENT_TASK, &envelope), Err(IpcSendError::ConnectionCongested));

        let fh = manager.accept_message_async(binding);
        let consumed = delivered_message(fh);
        assert_eq!(consumed.conn(), connection.pack() as u32);
        assert_eq!(consumed.data(), &[0u8; MESSAGE_PAYLOAD_BYTES]);

        assert_eq!(manager.send_message(CLIENT_TASK, &envelope), Ok(()));
    }

    #[test]
    fn accept_before_connect_delivers_message() {
        use crate::future::Future;
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
        let pattern: [u8; MESSAGE_PAYLOAD_BYTES] = core::array::from_fn(|i| (255 - i as u8) as u8);
        assert_eq!(manager.send_message(CLIENT_TASK, &Message::new(connection.pack() as u32, &pattern)), Ok(()));

        let delivered = delivered_message(fh);
        assert_eq!(delivered.conn(), connection.pack() as u32);
        assert_eq!(delivered.data(), &pattern);
    }

    #[test]
    fn receive_on_dead_connection_yields_error_future() {
        use crate::future::Future;
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
        use crate::future::Future;
        let mut manager = manager_with_notifier();

        let fh = manager.accept_message_async(IpcBindingHandle::new(99, 1));
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
    fn send_on_error_bit_conn_word_rejected() {
        let mut manager = manager_with_notifier();
        let (_binding, connection) = bind_and_connect(&mut manager, "MSG-GARBAGE");
        let malformed_conn = (system::error::ERROR_BIT | connection.pack()) as u32;
        assert_eq!(
            manager.send_message(CLIENT_TASK, &Message::new(malformed_conn, &[0u8; MESSAGE_PAYLOAD_BYTES])),
            Err(IpcSendError::ConnectionNotFound)
        );
    }

    #[test]
    fn disconnect_drops_queued_envelopes() {
        use crate::future::Future;
        let mut manager = manager_with_notifier();
        let (_binding, connection) = bind_and_connect(&mut manager, "MSG-DRAIN");

        assert_eq!(
            manager.send_message(SERVER_TASK, &Message::new(connection.pack() as u32, &[0u8; MESSAGE_PAYLOAD_BYTES])),
            Ok(())
        );
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
    fn disconnect_gc_frees_objects_and_refunds_budget() {
        let mut manager = IpcManager::new();
        let (_binding, connection) = bind_and_connect(&mut manager, "DATA-GC");
        let chunk = vec![3u8; 64 * 1024];
        let first = manager.write_data(CLIENT_TASK, connection, &chunk).unwrap();
        let second = manager.write_data(CLIENT_TASK, connection, &chunk).unwrap();
        manager.disconnect(connection);

        let mut dst = [0u8; 16];
        assert_eq!(manager.read_data(SERVER_TASK, first, &mut dst), Err(DataError::NotFound));
        assert_eq!(manager.read_data(SERVER_TASK, second, &mut dst), Err(DataError::NotFound));

        let reconnected = manager.connect("DATA-GC", CLIENT_TASK).unwrap();
        for _ in 0..4 {
            assert!(manager.write_data(CLIENT_TASK, reconnected, &chunk).is_ok());
        }
    }

    #[test]
    fn write_on_dead_connection_is_connection_not_found() {
        let mut manager = IpcManager::new();
        let (_binding, connection) = bind_and_connect(&mut manager, "DATA-DEAD");
        manager.disconnect(connection);
        assert_eq!(
            manager.write_data(CLIENT_TASK, connection, b"payload"),
            Err(DataError::ConnectionNotFound)
        );
    }

    #[test]
    fn read_membership_rejects_non_client_intruder() {
        let mut manager = IpcManager::new();
        let (_binding, connection) = bind_and_connect(&mut manager, "DATA-MEMBER");
        let token = manager.write_data(SERVER_TASK, connection, b"server written").unwrap();

        let mut dst = [0u8; 16];
        assert_eq!(manager.read_data(INTRUDER, token, &mut dst), Err(DataError::NotReadable));

        let copied = manager.read_data(CLIENT_TASK, token, &mut dst).unwrap();
        assert_eq!(&dst[..copied], b"server written");
    }

    #[test]
    fn client_written_object_readable_by_server_task() {
        let mut manager = IpcManager::new();
        let (_binding, connection) = bind_and_connect(&mut manager, "DATA-PAIR");
        let token = manager.write_data(CLIENT_TASK, connection, b"client written").unwrap();

        let mut dst = [0u8; 16];
        let copied = manager.read_data(SERVER_TASK, token, &mut dst).unwrap();
        assert_eq!(&dst[..copied], b"client written");
    }
}
