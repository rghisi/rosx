use crate::future::FutureRegistry;
use crate::ipc::ipc_manager::IpcManager;
use crate::kernel_cell::KernelCell;
use crate::memory::memory_manager::{MEMORY_MANAGER, MemoryManager};
use crate::once::Once;
use crate::scheduler::fifo_strategy::FifoStrategy;
use crate::scheduler::Scheduler;
use crate::scheduler::TimerManager;
use crate::task_manager::TaskManager;
use crate::task::TaskState;
use crate::{ForCompletingExpiredTimers, ForManagingTasks, ForNotifyingFutures, ForWakingTasks};
use system::future::{Future, FutureHandle};
use system::ipc::{IpcMessage, IpcMessageFuture};

// === Noop implementations for use in init() ===

struct NoopContextSwitcher;
impl crate::ForSwitchingTaskContext for NoopContextSwitcher {
    fn switch_to_task(&self, handle: crate::task::TaskHandle) -> crate::SwitchOutcome {
        crate::SwitchOutcome::Unchanged(handle)
    }
}
static NOOP_CTX_SWITCHER: NoopContextSwitcher = NoopContextSwitcher;

struct NoopTimerHandler;
impl ForCompletingExpiredTimers for NoopTimerHandler {
    fn complete_timer_future(&self, _handle: FutureHandle) {}
}
static NOOP_TIMER_HNDLR: NoopTimerHandler = NoopTimerHandler;

struct NoopTimeSource;
impl crate::ForReadingSystemTime for NoopTimeSource {
    fn now(&self) -> u64 { 0 }
}
static NOOP_TIME_SRC: NoopTimeSource = NoopTimeSource;

struct NoopTimerExpiry;
impl crate::ForExpiringTimers for NoopTimerExpiry {
    fn pop_expired(&self, _now: u64) -> Option<alloc::vec::Vec<FutureHandle>> { None }
}
static NOOP_TIMER_EXPIRY: NoopTimerExpiry = NoopTimerExpiry;

struct NoopInterruptHandler;
impl crate::ForHandlingHardwareInterrupts for NoopInterruptHandler {
    fn handle(&self, _interrupt: crate::messages::HardwareInterrupt) {}
}
static NOOP_INTR_HNDLR: NoopInterruptHandler = NoopInterruptHandler;
use alloc::boxed::Box;
use alloc::vec::Vec;

// === Port implementations (UseCases) ===

struct SchedulerWakerUseCase {
    scheduler: &'static KernelCell<Scheduler>,
}

impl ForWakingTasks for SchedulerWakerUseCase {
    fn wake_tasks(&self, handles: Vec<crate::task::TaskHandle>) {
        self.scheduler.borrow_mut().wake_tasks(handles);
    }
}

pub(crate) struct FutureRegistryNotifierUseCase {
    pub(crate) future_registry: &'static KernelCell<FutureRegistry>,
}

impl ForNotifyingFutures for FutureRegistryNotifierUseCase {
    fn register(&self, future: Box<dyn Future + Send + Sync>) -> Option<FutureHandle> {
        self.future_registry.borrow_mut().register(future)
    }

    fn notify(&self, handle: FutureHandle) {
        self.future_registry.borrow_mut().notify(handle);
    }

    fn complete_ipc_message(&self, handle: FutureHandle, message: IpcMessage) {
        if let Ok(future_box) = self.future_registry.borrow_mut().borrow_mut(handle) {
            if let Some(ipc_future) = future_box.as_any_mut().downcast_mut::<IpcMessageFuture>() {
                ipc_future.complete(message);
            }
        }
    }
}

struct FutureRegistryTimerUseCase {
    future_registry: &'static KernelCell<FutureRegistry>,
}

impl ForCompletingExpiredTimers for FutureRegistryTimerUseCase {
    fn complete_timer_future(&self, handle: FutureHandle) {
        if let Ok(future) = self.future_registry.borrow_mut().borrow_mut(handle) {
            future.complete();
        }
        self.future_registry.borrow_mut().notify(handle);
    }
}

/// Wrapper that delegates task management to a leaked KernelCell<TaskManager>.
struct StaticTaskManager {
    inner: &'static KernelCell<TaskManager>,
}

impl ForManagingTasks for StaticTaskManager {
    fn get_state(&self, handle: crate::task::TaskHandle) -> TaskState {
        self.inner.borrow().get_state(handle)
    }
    fn set_state(&self, handle: crate::task::TaskHandle, state: TaskState) {
        self.inner.borrow_mut().set_state(handle, state);
    }
    fn remove_task(&self, handle: crate::task::TaskHandle) {
        self.inner.borrow_mut().remove_task(handle);
    }
}

// === KernelServices ===

pub(crate) struct KernelServices {
    pub(crate) task_manager: &'static KernelCell<TaskManager>,
    pub(crate) future_registry: &'static KernelCell<FutureRegistry>,
    pub(crate) ipc_manager: &'static KernelCell<IpcManager>,
    pub(crate) timer_manager: &'static KernelCell<TimerManager>,
    pub(crate) scheduler: &'static KernelCell<Scheduler>,
    pub(crate) memory_manager: &'static MemoryManager,
    pub(crate) timer_handler: &'static dyn ForCompletingExpiredTimers,
}

static KERNEL_SERVICES: Once<KernelServices> = Once::new();

pub(crate) fn init() {
    let create_services = || {
        // Leak all KernelCells so the use cases can hold safe &'static
        // references to them. KernelCell<T> contains an UnsafeCell<T> with
        // no Drop, so leaking the wrapper is safe — the T lives for the
        // duration of the process (KERNEL_SERVICES is a static).
        let task_manager = Box::leak(Box::new(KernelCell::new(TaskManager::new())));
        let task_manager_wrapper: &'static dyn ForManagingTasks = Box::leak(Box::new(StaticTaskManager {
            inner: task_manager,
        })) as &'static dyn ForManagingTasks;
        let scheduler_cell = Box::leak(Box::new(KernelCell::new(Scheduler::new_full(
            FifoStrategy::new(),
            &NOOP_CTX_SWITCHER,
            &NOOP_TIMER_HNDLR,
            &NOOP_TIME_SRC,
            &NOOP_TIMER_EXPIRY,
            &NOOP_INTR_HNDLR,
            task_manager_wrapper,
        ))));
        let wake_controller = Box::leak(Box::new(SchedulerWakerUseCase {
            scheduler: scheduler_cell,
        })) as &'static dyn ForWakingTasks;
        let future_registry = Box::leak(Box::new(KernelCell::new(
            FutureRegistry::new_with_wake_controller(wake_controller),
        )));
        let notifier = Box::leak(Box::new(FutureRegistryNotifierUseCase {
            future_registry: future_registry,
        })) as &'static dyn ForNotifyingFutures;
        let timer_handler = Box::leak(Box::new(FutureRegistryTimerUseCase {
            future_registry: future_registry,
        })) as &'static dyn ForCompletingExpiredTimers;
        let timer_manager = Box::leak(Box::new(KernelCell::new(TimerManager::new())));
        let ipc_manager = Box::leak(Box::new(KernelCell::new(IpcManager::new_with_notifier(notifier))));

        KernelServices {
            task_manager,
            future_registry,
            ipc_manager,
            timer_manager,
            scheduler: scheduler_cell,
            memory_manager: &MEMORY_MANAGER,
            timer_handler,
        }
    };

    #[cfg(not(test))]
    KERNEL_SERVICES.call_once(create_services);

    #[cfg(test)]
    {
        static TEST_INIT: std::sync::OnceLock<()> = std::sync::OnceLock::new();
        TEST_INIT.get_or_init(|| {
            KERNEL_SERVICES.call_once(create_services);
        });
    }
}

pub(crate) fn services() -> &'static KernelServices {
    KERNEL_SERVICES.get().expect("KernelServices not initialized")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::task::Task;

    #[test]
    fn init_and_access_services() {
        init();
        let s = services();

        let task = Task::new("test", 0x1000, 0);
        let handle = s.task_manager.borrow_mut().add_task(task).unwrap();
        assert_eq!(s.task_manager.borrow().get_state(handle), crate::task::TaskState::Created);

        let future = alloc::boxed::Box::new(crate::future::TaskCompletionFuture::new(handle));
        let fh = s.future_registry.borrow_mut().register(future);
        assert!(fh.is_some());
    }
}
