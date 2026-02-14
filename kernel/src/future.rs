use crate::syscall;
use alloc::boxed::Box;
use system::future::FutureHandle;
use collections::generational_arena::GenArena;
use crate::kernel::TASK_MANAGER;
use crate::task::TaskHandle;

pub trait Future: Send + Sync {
    fn is_completed(&self) -> bool;
}

pub struct TimeFuture {
    completion_timestamp: u64,
}

impl TimeFuture {
    pub fn new(ms: u64) -> TimeFuture {
        TimeFuture {
            completion_timestamp: syscall::get_system_time() + ms,
        }
    }
}
impl Future for TimeFuture {
    fn is_completed(&self) -> bool {
        syscall::get_system_time() > self.completion_timestamp
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
        TASK_MANAGER.borrow().get_state(self.task_handle) == crate::task::TaskState::Terminated
    }
}

pub struct FutureRegistry {
    arena: GenArena<Box<dyn Future + Send + Sync>, u32, u32>,
}

impl Default for FutureRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl FutureRegistry {
    pub fn new() -> Self {
        Self {
            arena: GenArena::new(10),
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

    pub fn remove(&mut self, handle: FutureHandle) {
        let _ = self.arena.remove(handle);
    }
}


