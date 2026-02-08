use crate::syscall;
use alloc::boxed::Box;
use spin::Mutex;
use system::future::FutureHandle;
use crate::generational_arena::{GenArena, Handle};
use crate::kernel::TASK_MANAGER;
use crate::task::TaskHandle;

pub trait Future {
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

impl FutureRegistry {
    pub fn new() -> Self {
        Self {
            arena: Mutex::new(GenArena::new(1024)),
        }
    }

    pub fn register(&self, future: Box<dyn Future + Send + Sync>) -> Option<FutureHandle> {
        let mut arena = self.arena.lock();
        if let Ok(handle) = arena.add(future) {
            // Pack index (u32) and generation (u32) into u64
            let packed = (handle.index as u64) << 32 | (handle.generation as u64);
            Some(FutureHandle(packed))
        } else {
            None
        }
    }

    pub fn get(&self, handle: FutureHandle) -> Option<bool> {
        let index = (handle.0 >> 32) as u32;
        let generation = handle.0 as u32;
        let arena_handle = Handle { index, generation };
        
        let mut arena = self.arena.lock();
        if let Ok(future) = arena.borrow_mut(arena_handle) {
             Some(future.is_completed())
        } else {
            None
        }
    }
    
    // Helper to get the actual future object (used for wait)
    // Actually, wait implementation in kernel takes ownership or mutable borrow?
    // Kernel::wait takes Box<dyn Future>.
    // If we use registry, we don't want to extract it, just check it.
    // But Kernel::wait logic: 
    // self.execution_state.block_current_task();
    // self.main_thread.push_blocked(task_handle, future);
    
    // We need to change Kernel::wait to take a reference or something. 
    // Or, we create a wrapper future that queries the registry!
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

