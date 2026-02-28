use alloc::boxed::Box;
use core::any::Any;
use system::future::FutureHandle;
use system::future::Future;
use collections::generational_arena::{Error, GenArena};
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
    arena: GenArena<Box<dyn Future + Send + Sync>, 256>,
}

impl Default for FutureRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl FutureRegistry {
    pub fn new() -> Self {
        Self {
            arena: GenArena::new(),
        }
    }

    pub fn register(&mut self, future: Box<dyn Future + Send + Sync>) -> Option<FutureHandle> {
        self.arena.add(future).ok()
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

}


