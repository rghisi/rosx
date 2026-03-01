use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use core::any::Any;
use system::future::FutureHandle;
use system::future::Future;
use collections::generational_arena::{Error, GenerationalArena};
use crate::kernel::kernel;
use crate::kernel_services::services;
use crate::task::TaskHandle;

pub struct TimeFuture {
    duration: core::time::Duration,
    completion_timestamp: u64,
}

impl TimeFuture {
    pub fn new(ms: u64) -> TimeFuture {
        TimeFuture {
            duration: core::time::Duration::from_millis(ms),
            completion_timestamp: kernel().get_system_time() + ms,
        }
    }

    pub fn duration(&self) -> core::time::Duration {
        self.duration
    }
}
impl Future for TimeFuture {
    fn is_completed(&self) -> bool {
        kernel().get_system_time() > self.completion_timestamp
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn complete(&mut self) {
        self.completion_timestamp = 0;
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

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn complete(&mut self) {
    }
}


pub struct FutureRegistry {
    pub(crate) arena: GenerationalArena<Box<dyn Future + Send + Sync>, 1024>,
    subscriptions: BTreeMap<FutureHandle, TaskHandle>,
    completed: BTreeMap<FutureHandle, bool>,
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
            subscriptions: BTreeMap::new(),
            completed: BTreeMap::new(),
        }
    }

    pub fn register(&mut self, future: Box<dyn Future + Send + Sync>) -> Option<FutureHandle> {
        let is_completed = future.is_completed();
        let handle = self.arena.add(future).ok()?;
        if is_completed {
            self.completed.insert(handle, true);
        }
        Some(handle)
    }

    pub fn get(&mut self, handle: FutureHandle) -> Option<bool> {
        if let Ok(future) = self.arena.borrow_mut(handle) {
            Some(future.is_completed())
        } else {
            None
        }
    }

    pub fn consume(&mut self, handle: FutureHandle) -> Result<Box<dyn Future + Send + Sync>, Error> {
        self.subscriptions.remove(&handle);
        self.completed.remove(&handle);
        self.arena.remove(handle)
    }

    pub fn replace(&mut self, handle: FutureHandle, future: Box<dyn Future + Send + Sync>) -> Result<FutureHandle, Error> {
        self.arena.replace(handle, future)
    }

    pub fn subscribe(&mut self, future_handle: FutureHandle, task_handle: TaskHandle) {
        self.subscriptions.insert(future_handle, task_handle);
    }

    pub fn complete(&mut self, handle: FutureHandle) -> Option<TaskHandle> {
        if let Ok(future) = self.arena.borrow_mut(handle) {
            future.complete();
        }
        self.completed.insert(handle, true);
        let task_handle = self.subscriptions.remove(&handle);
        if let Some(_th) = task_handle {
            #[cfg(not(test))]
            kernel().wake_task(_th);
        }
        task_handle
    }

    pub fn is_completed(&self, handle: FutureHandle) -> bool {
        self.completed.contains_key(&handle)
    }

    pub fn is_subscribed(&self, handle: FutureHandle) -> bool {
        self.subscriptions.contains_key(&handle)
    }

}

#[cfg(test)]
mod tests {
    use super::*;
    use collections::generational_arena::Handle;

    struct MockFuture;
    impl Future for MockFuture {
        fn is_completed(&self) -> bool { false }
        fn as_any(&self) -> &dyn Any { self }
        fn as_any_mut(&mut self) -> &mut dyn Any { self }
        fn complete(&mut self) {}
    }

    #[test]
    fn test_subscribe_and_complete() {
        let mut registry = FutureRegistry::new();
        let future = Box::new(MockFuture);
        let fh = registry.register(future).unwrap();
        let th = Handle::new(1, 1);

        registry.subscribe(fh, th);
        let waiter = registry.complete(fh);
        assert_eq!(waiter, Some(th));
        assert!(registry.is_completed(fh));
    }

    #[test]
    fn test_already_completed_registration() {
        struct CompletedMock;
        impl Future for CompletedMock {
            fn is_completed(&self) -> bool { true }
            fn as_any(&self) -> &dyn Any { self }
            fn as_any_mut(&mut self) -> &mut dyn Any { self }
            fn complete(&mut self) {}
        }
        let mut registry = FutureRegistry::new();
        let future = Box::new(CompletedMock);
        let fh = registry.register(future).unwrap();
        assert!(registry.is_completed(fh));
    }
}
