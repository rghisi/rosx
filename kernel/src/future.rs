use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use core::any::Any;
use system::future::{Future, FutureHandle, FutureResult};
use collections::generational_arena::{Error, GenerationalArena};
use crate::kernel_services::services;
use crate::task::TaskHandle;

pub struct TimeFuture {
    completed: bool,
}

impl TimeFuture {
    pub fn new() -> TimeFuture {
        TimeFuture { completed: false }
    }
}
impl Future for TimeFuture {
    fn is_completed(&self) -> bool {
        self.completed
    }

    fn complete(&mut self) {
        self.completed = true;
    }

    fn into_result(self: Box<Self>) -> FutureResult {
        FutureResult::Void
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn into_any(self: Box<Self>) -> Box<dyn Any + Send + Sync> {
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

    fn into_result(self: Box<Self>) -> FutureResult {
        FutureResult::Void
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn into_any(self: Box<Self>) -> Box<dyn Any + Send + Sync> {
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

    pub fn borrow_mut(&mut self, handle: FutureHandle) -> Result<&mut Box<dyn Future + Send + Sync>, Error> {
        self.arena.borrow_mut(handle)
    }

    pub fn replace(&mut self, handle: FutureHandle, future: Box<dyn Future + Send + Sync>) -> Result<FutureHandle, Error> {
        self.arena.replace(handle, future)
    }

    pub fn notify(&mut self, handle: FutureHandle) {
        let waiters = self.waiters.remove(&handle).unwrap_or_default();
        services().scheduler.borrow_mut().wake_tasks(waiters);
    }

}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel_services::{init as init_services, services};
    use crate::task::Task;
    use crate::task::TaskState;
    use std::sync::Once;

    static INIT: Once = Once::new();

    fn setup() {
        INIT.call_once(|| init_services());
    }

    struct DummyFuture;
    impl Future for DummyFuture {
        fn is_completed(&self) -> bool {
            false
        }
        fn into_result(self: Box<Self>) -> FutureResult {
            FutureResult::Void
        }
        fn as_any(&self) -> &dyn Any {
            self
        }
        fn as_any_mut(&mut self) -> &mut dyn Any {
            self
        }
        fn into_any(self: Box<Self>) -> Box<dyn Any + Send + Sync> {
            self
        }
    }

    #[test]
    fn notify_moves_waiters_to_ready() {
        setup();

        let task = Task::new("T", 0x1000, 0);
        let handle = services().task_manager.borrow_mut().add_task(task).unwrap();
        services().task_manager.borrow_mut().set_state(handle, TaskState::Blocked);

        let future_handle = services().future_registry.borrow_mut().register(Box::new(DummyFuture)).unwrap();
        services().future_registry.borrow_mut().register_waiter(future_handle, handle);

        services().future_registry.borrow_mut().notify(future_handle);

        assert_eq!(services().task_manager.borrow().get_state(handle), TaskState::Ready);
    }

    #[test]
    fn notify_does_not_double_wake() {
        setup();

        let task = Task::new("T", 0x1000, 0);
        let handle = services().task_manager.borrow_mut().add_task(task).unwrap();
        services().task_manager.borrow_mut().set_state(handle, TaskState::Blocked);

        let future_handle = services().future_registry.borrow_mut().register(Box::new(DummyFuture)).unwrap();
        services().future_registry.borrow_mut().register_waiter(future_handle, handle);

        // First notify moves task to Ready
        services().future_registry.borrow_mut().notify(future_handle);
        assert_eq!(services().task_manager.borrow().get_state(handle), TaskState::Ready);

        // Second notify does nothing (no waiters left)
        services().future_registry.borrow_mut().notify(future_handle);
        assert_eq!(services().task_manager.borrow().get_state(handle), TaskState::Ready);
    }

    // === TimeFuture tests ===

    #[test]
    fn time_future_starts_not_completed() {
        let future = TimeFuture::new();
        assert!(!future.is_completed());
    }

    #[test]
    fn time_future_complete_flips_flag() {
        let mut future = TimeFuture::new();
        future.complete();
        assert!(future.is_completed());
    }

    #[test]
    fn time_future_multiple_completes_are_idempotent() {
        let mut future = TimeFuture::new();
        future.complete();
        future.complete();
        future.complete();
        assert!(future.is_completed());
    }

    #[test]
    fn future_registry_can_complete_time_future_via_trait() {
        setup();

        let mut registry = FutureRegistry::new();
        let time_future = Box::new(TimeFuture::new());
        let handle = registry.register(time_future).unwrap();

        assert!(!registry.get(handle).unwrap());

        // Complete through the trait object (simulates timer firing)
        if let Ok(future) = registry.borrow_mut(handle) {
            future.complete();
        }

        assert!(registry.get(handle).unwrap());
    }

    #[test]
    fn complete_default_noop_does_not_affect_other_futures() {
        setup();

        let mut registry = FutureRegistry::new();
        let dummy = Box::new(DummyFuture);
        let handle = registry.register(dummy).unwrap();

        assert!(!registry.get(handle).unwrap());

        // Calling complete() on a non-TimeFuture is a no-op
        if let Ok(future) = registry.borrow_mut(handle) {
            future.complete();
        }

        assert!(!registry.get(handle).unwrap());
    }
}


