use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use collections::generational_arena::{GenerationalArena, Handle};
use system::future::FutureHandle;
use system::ipc::{IpcMessage, IpcMessageFuture, IpcReceiveError};
use crate::ipc::mailbox::Mailbox;
use crate::kernel_services::services;

pub(crate) type MailboxHandle = Handle;

pub(crate) struct MailboxManager {
    mailboxes: GenerationalArena<Mailbox, 256>,
    waiters: BTreeMap<MailboxHandle, Vec<FutureHandle>>,
}

impl MailboxManager {
    pub(crate) fn new() -> Self {
        Self {
            mailboxes: GenerationalArena::new(),
            waiters: BTreeMap::new(),
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

    pub(crate) fn pop_front_async(&mut self, handle: MailboxHandle) -> IpcMessageFuture {
        if let Ok(mailbox) = self.mailboxes.borrow_mut(handle) {
            if let Some(msg) = mailbox.pop_front() {
                IpcMessageFuture::with_message(msg)
            } else {
                let future = alloc::boxed::Box::new(IpcMessageFuture::new());
                let future_handle = services().future_registry.borrow_mut().register(future).unwrap();
                self.waiters.entry(handle).or_default().push(future_handle);
                IpcMessageFuture::with_handle(future_handle)
            }
        } else {
            IpcMessageFuture::with_error(IpcReceiveError::MailboxNotAvailable)
        }
    }

    fn notify_waiters(&mut self, handle: MailboxHandle) {
        if let Some(waiters) = self.waiters.get_mut(&handle) {
            while !waiters.is_empty() {
                if let Ok(mailbox) = self.mailboxes.borrow_mut(handle) {
                    if let Some(msg) = mailbox.pop_front() {
                        let fh = waiters.remove(0);
                        if let Ok(future_box) = services().future_registry.borrow_mut().borrow_mut(fh) {
                            if let Some(ipc_future) = future_box.as_any_mut().downcast_mut::<IpcMessageFuture>() {
                                ipc_future.complete(msg);
                            }
                        }
                        services().future_registry.borrow_mut().notify(fh);
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
            data: 123,
        };
        future.complete(msg);
        assert!(future.is_completed());
        assert_eq!(future.get_message().unwrap().data, 123);
    }

    #[test]
    fn test_pop_front_async() {
        init_services();
        let mut manager = MailboxManager::new();
        let handle = manager.create();
        
        // Case 1: Empty mailbox -> Pending
        let future = manager.pop_front_async(handle);
        assert!(future.get_message().is_none());
        let fh = future.get_handle().expect("Expected pending future handle");
        
        // Case 2: Data pushed -> Waiter notified
        let msg = IpcMessage {
            connection_handle: Handle::new(1, 1),
            data: 456,
        };
        manager.push_back(handle, msg).unwrap();
        
        // Verify that the future now has the message
        let mut registry = services().future_registry.borrow_mut();
        let future_box = registry.borrow_mut(fh).unwrap();
        let ipc_future = future_box.as_any().downcast_ref::<IpcMessageFuture>().unwrap();
        assert_eq!(ipc_future.get_message().unwrap().data, 456);
    }

    #[test]
    fn test_push_back_notifies_waiters() {
        init_services();
        let mut manager = MailboxManager::new();
        let handle = manager.create();
        
        // 1. Create a blocked task
        let task = crate::task::Task::new("waiter", 0x1000, 0);
        let task_handle = services().task_manager.borrow_mut().add_task(task).unwrap();
        services().task_manager.borrow_mut().set_state(task_handle, crate::task::TaskState::Blocked);
        
        // 2. pop_front_async to get a future
        let future_handle = manager.pop_front_async(handle)
            .get_handle().expect("Expected pending future handle");
        
        // 3. Register task as waiter for that future
        services().future_registry.borrow_mut().register_waiter(future_handle, task_handle);
        
        // 4. push_back should notify and wake the task
        let msg = IpcMessage {
            connection_handle: Handle::new(1, 1),
            data: 789,
        };
        manager.push_back(handle, msg).unwrap();
        
        assert_eq!(services().task_manager.borrow().get_state(task_handle), crate::task::TaskState::Ready);
    }
}
