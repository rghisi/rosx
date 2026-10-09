use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use alloc::boxed::Box;
use collections::generational_arena::{GenerationalArena, Handle};
use system::future::FutureHandle;
use system::ipc::{IpcMessage, IpcMessageFuture, IpcReceiveError};
use crate::ipc::mailbox::Mailbox;
use crate::ports::driven::ForNotifyingFutures;

struct NoopNotifier;
impl ForNotifyingFutures for NoopNotifier {
    fn register(&self, _future: Box<dyn system::future::Future + Send + Sync>) -> Option<system::future::FutureHandle> {
        None
    }
    fn notify(&self, _handle: system::future::FutureHandle) {}
    fn complete_ipc_message(&self, _handle: system::future::FutureHandle, _message: IpcMessage) {}
}

static NOOP_NOTIFIER: NoopNotifier = NoopNotifier;

pub(crate) type MailboxHandle = Handle;

pub(crate) struct MailboxManager {
    mailboxes: GenerationalArena<Mailbox, 256>,
    waiters: BTreeMap<MailboxHandle, Vec<FutureHandle>>,
    notifier: &'static dyn ForNotifyingFutures,
}

impl MailboxManager {
    pub(crate) fn new() -> Self {
        Self {
            mailboxes: GenerationalArena::new(),
            waiters: BTreeMap::new(),
            notifier: &NOOP_NOTIFIER,
        }
    }

    pub(crate) fn new_with_notifier(notifier: &'static dyn ForNotifyingFutures) -> Self {
        Self {
            mailboxes: GenerationalArena::new(),
            waiters: BTreeMap::new(),
            notifier,
        }
    }

    pub(crate) fn create(&mut self) -> MailboxHandle {
        self.mailboxes.add(Mailbox::new()).unwrap()
    }

    pub(crate) fn remove(&mut self, handle: MailboxHandle) {
        let _ = self.mailboxes.remove(handle);
        self.waiters.remove(&handle);
    }

    pub(crate) fn push_back(&mut self, handle: MailboxHandle, message: IpcMessage) {
        if let Ok(mailbox) = self.mailboxes.borrow_mut(handle) {
            mailbox.push_back(message);
            self.notify_waiters(handle);
        }
    }

    pub(crate) fn pop_front_async(&mut self, handle: MailboxHandle) -> FutureHandle {
        if let Ok(mailbox) = self.mailboxes.borrow_mut(handle) {
            if let Some(msg) = mailbox.pop_front() {
                self.notifier
                    .register(Box::new(IpcMessageFuture::with_message(msg)))
                    .unwrap()
            } else {
                let fh = self
                    .notifier
                    .register(Box::new(IpcMessageFuture::new()))
                    .unwrap();
                self.waiters.entry(handle).or_default().push(fh);
                fh
            }
        } else {
            self.notifier
                .register(Box::new(IpcMessageFuture::with_error(IpcReceiveError::MailboxNotAvailable)))
                .unwrap()
        }
    }

    fn notify_waiters(&mut self, handle: MailboxHandle) {
        if let Some(waiters) = self.waiters.get_mut(&handle) {
            while !waiters.is_empty() {
                if let Ok(mailbox) = self.mailboxes.borrow_mut(handle) {
                    if let Some(msg) = mailbox.pop_front() {
                        let fh = waiters.remove(0);
                        self.notifier.complete_ipc_message(fh, msg);
                        self.notifier.notify(fh);
                    } else {
                        break;
                    }
                } else {
                    break;
                }
            }
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
    use alloc::boxed::Box;

    #[test]
    fn test_create_remove() {
        let mut manager = MailboxManager::new();
        let handle = manager.create();
        
        // Check that we can borrow it (indirectly verifying it's valid)
        assert!(manager.mailboxes.borrow(handle).is_ok());
        
        manager.remove(handle);
        
        // Check that it's now invalid
        assert!(manager.mailboxes.borrow(handle).is_err());
    }

    #[test]
    fn test_ipc_message_future() {
        use system::future::Future;
        let mut future = IpcMessageFuture::new();
        assert!(!future.is_completed());
        let msg = IpcMessage {
            connection_handle: Handle::new(1, 1),
            buffer_handle: Handle::new(7, 1),
        };
        future.complete(msg);
        assert!(future.is_completed());
        assert_eq!(future.result().unwrap().buffer_handle, Handle::new(7, 1));
    }

    #[test]
    fn test_pop_front_async() {
        init_services();
        let notifier: &'static dyn ForNotifyingFutures =
            Box::leak(Box::new(FutureRegistryNotifierUseCase {
                future_registry: services().future_registry,
            })) as &'static dyn ForNotifyingFutures;
        let mut manager = MailboxManager::new_with_notifier(notifier);
        let handle = manager.create();
        
        // Case 1: Empty mailbox -> Pending
        let fh = manager.pop_front_async(handle);
        
        // Case 2: Data pushed -> Waiter notified
        let msg = IpcMessage {
            connection_handle: Handle::new(1, 1),
            buffer_handle: Handle::new(7, 2),
        };
        manager.push_back(handle, msg);

        // Verify that the future now has the message
        let mut registry = services().future_registry.borrow_mut();
        let future_box = registry.borrow_mut(fh).unwrap();
        let ipc_future = future_box.as_any().downcast_ref::<IpcMessageFuture>().unwrap();
        assert_eq!(ipc_future.result().unwrap().buffer_handle, Handle::new(7, 2));
    }

    #[test]
    fn test_push_back_notifies_waiters() {
        init_services();
        crate::scheduler::wire_scheduler_for_tests();
        let notifier: &'static dyn ForNotifyingFutures =
            Box::leak(Box::new(FutureRegistryNotifierUseCase {
                future_registry: services().future_registry,
            })) as &'static dyn ForNotifyingFutures;
        let mut manager = MailboxManager::new_with_notifier(notifier);
        let handle = manager.create();
        
        // 1. Create a blocked task
        let task = crate::task::Task::new("waiter", 0x1000, 0);
        let task_handle = services().task_manager.borrow_mut().add_task(task).unwrap();
        services().task_manager.borrow_mut().set_state(task_handle, crate::task::TaskState::Blocked);
        
        // 2. pop_front_async to get a future
        let future_handle = manager.pop_front_async(handle);
        
        // 3. Register task as waiter for that future
        services().future_registry.borrow_mut().register_waiter(future_handle, task_handle);
        
        // 4. push_back should notify and wake the task
        let msg = IpcMessage {
            connection_handle: Handle::new(1, 1),
            buffer_handle: Handle::new(7, 3),
        };
        manager.push_back(handle, msg);

        assert_eq!(services().task_manager.borrow().get_state(task_handle), crate::task::TaskState::Ready);
    }
}
