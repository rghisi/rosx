use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use core::any::Any;
use system::future::FutureHandle;
use system::future::Future;
use collections::generational_arena::{Error, GenerationalArena};
use crate::kernel::kernel;
use crate::kernel_services::services;
use crate::task::TaskHandle;

pub struct TimeFuture {
    completion_timestamp: u64,
}

impl TimeFuture {
    pub fn new(ms: u64) -> TimeFuture {
        TimeFuture {
            completion_timestamp: kernel().get_system_time() + ms,
        }
    }
}
impl Future for TimeFuture {
    fn is_completed(&self) -> bool {
        kernel().get_system_time() > self.completion_timestamp
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

pub struct TaskCompletionFuture {
    task_handle: TaskHandle,
}

impl TaskCompletionFuture {
    pub fn new(task_handle: TaskHandle) -> Self {
        Self { task_handle }
    }
}

impl Future for TaskCompletionFuture {
    fn is_completed(&self) -> bool {
        services().task_manager.borrow().get_state(self.task_handle) == crate::task::TaskState::Terminated
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

pub(crate) struct TaskFuture {
    pub(crate) task_handle: TaskHandle,
    pub(crate) future_handle: FutureHandle,
}

impl TaskFuture {
    pub(crate) fn is_completed(&self) -> bool {
        services().future_registry.borrow_mut().get(self.future_handle).unwrap_or(true)
    }
}

pub struct FutureRegistry {
    arena: GenerationalArena<Box<dyn Future + Send + Sync>, 1024>,
    waiters: BTreeMap<FutureHandle, Vec<TaskHandle>>,
}

impl Default for FutureRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl FutureRegistry {
    pub fn new() -> Self {
        Self {
            arena: GenerationalArena::new(),
            waiters: BTreeMap::new(),
        }
    }

    pub fn register(&mut self, future: Box<dyn Future + Send + Sync>) -> Option<FutureHandle> {
        self.arena.add(future).ok()
    }

    pub fn register_waiter(&mut self, future_handle: FutureHandle, task_handle: TaskHandle) {
        self.waiters.entry(future_handle).or_default().push(task_handle);
    }

    pub fn get(&mut self, handle: FutureHandle) -> Option<bool> {
        if let Ok(future) = self.arena.borrow_mut(handle) {
            Some(future.is_completed())
        } else {
            None
        }
    }

    pub fn consume(&mut self, handle: FutureHandle) -> Result<Box<dyn Future + Send + Sync>, Error> {
        self.arena.remove(handle)
    }

    pub fn replace(&mut self, handle: FutureHandle, future: Box<dyn Future + Send + Sync>) -> Result<FutureHandle, Error> {
        self.arena.replace(handle, future)
    }

    pub fn notify(&mut self, handle: FutureHandle) -> Vec<TaskHandle> {
        self.waiters.remove(&handle).unwrap_or_default()
    }

}

#[cfg(test)]
mod tests {
    use super::*;

    struct DummyFuture;
    impl Future for DummyFuture {
        fn is_completed(&self) -> bool {
            false
        }
        fn as_any(&self) -> &dyn Any {
            self
        }
    }

    fn make_registry() -> FutureRegistry {
        FutureRegistry::new()
    }

    #[test]
    fn notify_wakes_registered_waiters() {
        let mut registry = make_registry();

        let future_handle = registry.register(Box::new(DummyFuture)).unwrap();
        registry.register_waiter(future_handle, TaskHandle::new(1, 0));

        let woken = registry.notify(future_handle);

        assert_eq!(woken, vec![TaskHandle::new(1, 0)]);
    }

    #[test]
    fn notify_returns_empty_when_no_waiters() {
        let mut registry = make_registry();

        let future_handle = registry.register(Box::new(DummyFuture)).unwrap();
        let woken = registry.notify(future_handle);

        assert!(woken.is_empty());
    }

    #[test]
    fn notify_removes_waiters_from_map() {
        let mut registry = make_registry();

        let future_handle = registry.register(Box::new(DummyFuture)).unwrap();
        registry.register_waiter(future_handle, TaskHandle::new(2, 0));

        let woken_first = registry.notify(future_handle);
        assert_eq!(woken_first.len(), 1);
        assert_eq!(woken_first[0], TaskHandle::new(2, 0));

        let woken_second = registry.notify(future_handle);
        assert!(woken_second.is_empty());
    }

    #[test]
    fn notify_notifies_waiter_on_future() {
        let mut registry = make_registry();

        let future_handle = registry.register(Box::new(DummyFuture)).unwrap();
        registry.register_waiter(future_handle, TaskHandle::new(3, 0));

        let woken = registry.notify(future_handle);

        assert_eq!(woken.len(), 1);
        assert_eq!(woken[0], TaskHandle::new(3, 0));
    }
}


