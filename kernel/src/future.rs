use crate::syscall;
use alloc::boxed::Box;
use spin::Mutex;
use system::future::FutureHandle;
use crate::generational_arena::GenArena;
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
        let m = TASK_MANAGER.lock();
        let mm = m.borrow();
        mm.get_state(self.task_handle) == crate::task::TaskState::Terminated
    }
}

pub struct FutureRegistry {
    arena: Mutex<GenArena<Box<dyn Future + Send + Sync>, u32, u32>>,
}

impl Default for FutureRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl FutureRegistry {
    pub fn new() -> Self {
        Self {
            arena: Mutex::new(GenArena::new(10)),
        }
    }

    pub fn register(&self, future: Box<dyn Future + Send + Sync>) -> Option<FutureHandle> {
        let mut arena = self.arena.lock();
        arena.add(future).ok()
    }

    pub fn get(&self, handle: FutureHandle) -> Option<bool> {
        let mut arena = self.arena.lock();
        if let Ok(future) = arena.borrow_mut(handle) {
             Some(future.is_completed())
        } else {
            None
        }
    }

    pub fn remove(&self, handle: FutureHandle) {
        let mut arena = self.arena.lock();
        let _ = arena.remove(handle);
    }
}

pub struct RegistryFuture {
    handle: FutureHandle,
    registry: &'static FutureRegistry,
}

impl RegistryFuture {
    pub fn new(handle: FutureHandle, registry: &'static FutureRegistry) -> Self {
        Self { handle, registry }
    }
}

impl Future for RegistryFuture {
    fn is_completed(&self) -> bool {
        self.registry.get(self.handle).unwrap_or(true) 
    }
}

